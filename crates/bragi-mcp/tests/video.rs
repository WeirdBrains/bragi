//! Video over MCP against the demo node, with `FakeSource` for the clip (no ffmpeg): the tool
//! table, `bragi_video_probe`, `bragi_video_frames` by count and by a model's limits, the `video`
//! input on `bragi_chat`, and the frames read back through weirdnet-mcp's client the way an MCP
//! client sees them (`Client::call_tool_content`, `Content::from_json`, `decoded_bytes`), including
//! what the stdio line cap does to a big batch.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bragi_core::Core;
use bragi_core::video::{FakeSource, FrameSource, MediaLimits, ModelLimits};
use bragi_mcp::{BragiTools, Options};
use serde_json::{Value, json};
use weirdnet_mcp::client::{Client, ClientOptions, StdioTransport};
use weirdnet_mcp::{Content, Server};

type Rpc = Server<BragiTools>;

fn tools(core: &Core, root: &Path) -> BragiTools {
    let fake: bragi_mcp::video::VideoSourceFn = Arc::new(|_p: &Path| Ok(Box::new(FakeSource::new(42_000, true)) as Box<dyn FrameSource + Send>));
    tools_with(core, root, fake)
}

fn tools_with(core: &Core, root: &Path, open: bragi_mcp::video::VideoSourceFn) -> BragiTools {
    let journal = root.parent().unwrap().join("undo");
    let opts = Options { allow_deletes: false, allow_writes: false, root: root.to_path_buf(), timeout: Duration::from_secs(60), journal };
    BragiTools::new(core.clone(), opts).unwrap().with_video_source(open)
}

/// A clip whose frames are 64 KiB each, for the line cap.
struct BigFrames(FakeSource);

impl FrameSource for BigFrames {
    fn probe(&self) -> bragi_core::Result<bragi_core::video::Probe> {
        self.0.probe()
    }
    fn frame_at(&self, at_ms: u64, width: u32, height: u32) -> bragi_core::Result<bragi_core::video::Frame> {
        let mut f = self.0.frame_at(at_ms, width, height)?;
        f.bytes.resize(64 * 1024, 0);
        Ok(f)
    }
    fn scenes(&self, threshold: f32) -> bragi_core::Result<Vec<(u64, f32)>> {
        self.0.scenes(threshold)
    }
    fn audio(&self, start_ms: u64, len_ms: u64) -> bragi_core::Result<Vec<u8>> {
        self.0.audio(start_ms, len_ms)
    }
}

async fn call(rpc: &Rpc, name: &str, args: Value) -> Value {
    rpc.handle_async(None, json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}))
        .await
        .expect("a reply")["result"]
        .clone()
}

async fn setup() -> (tempfile::TempDir, Arc<bragi_core::demo::DemoNode>, Core, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let (node, core) = bragi_mcp::cli::demo(&tmp.path().join("bragi")).await.unwrap();
    let root = tmp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("clip.mp4"), b"not decoded: the fake source answers").unwrap();
    std::fs::write(tmp.path().join("outside.mp4"), b"x").unwrap();
    (tmp, node, core, root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_frames_and_chat_with_a_clip() {
    let (tmp, _node, core, root) = setup().await;
    let rpc = Arc::new(bragi_mcp::server(tools(&core, &root)));
    let list = rpc.handle_async(None, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await.unwrap();
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"bragi_video_probe") && names.contains(&"bragi_video_frames"), "{names:?}");
    let frames_tool = list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "bragi_video_frames").unwrap();
    assert_eq!(frames_tool["annotations"]["readOnlyHint"], true);

    // Probe: what the clip is, as text and structured.
    let r = call(&rpc, "bragi_video_probe", json!({"file": "clip.mp4"})).await;
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["duration_ms"], 42_000);
    assert_eq!(r["structuredContent"]["has_audio"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("1920x1080"));
    // Only files inside the project root.
    let r = call(&rpc, "bragi_video_probe", json!({"file": tmp.path().join("outside.mp4")})).await;
    assert_eq!(r["isError"], true, "{r}");

    // Frames by count: a text block with the time before each image; times in structuredContent.
    let r = call(&rpc, "bragi_video_frames", json!({"file": "clip.mp4", "count": 3, "max_edge": 320})).await;
    assert_eq!(r["isError"], false, "{r}");
    let content = r["content"].as_array().unwrap();
    assert_eq!(content.len(), 1 + 3 * 2);
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "[00:07.0]");
    assert_eq!(content[2]["type"], "image");
    assert_eq!(content[2]["mimeType"], "image/jpeg");
    assert!(content[2].get("annotations").is_none(), "Content::Image has no annotations in weirdnet-mcp 0.6.2");
    let frames = r["structuredContent"]["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[1]["at_ms"], 21_000);
    assert_eq!(frames[1]["label"], "[00:21.0]");
    assert_eq!(r["structuredContent"]["clip"]["width"], 1920);
    let r = call(&rpc, "bragi_video_frames", json!({"file": "clip.mp4", "count": 65})).await;
    assert_eq!(r["isError"], true, "more than 64 frames are refused: {r}");

    // By the model's limits: the demo pool reports a context but no image cost, so it refuses and
    // says what to set; never a default count.
    let model = core.state().catalog.default_model().unwrap().to_string();
    let r = call(&rpc, "bragi_video_frames", json!({"file": "clip.mp4"})).await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("image cost"), "{r}");
    core.set_video_override(&model, ModelLimits::new(None, MediaLimits::new().with_image_tokens(256).with_max_images(6))).unwrap();
    let r = call(&rpc, "bragi_video_frames", json!({"file": "clip.mp4"})).await;
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["frames"].as_array().unwrap().len(), 6, "max_images caps the frames: {r}");

    // bragi_chat with `video`: one attachment holding the frames, kept with the session.
    let r = call(&rpc, "bragi_chat", json!({"prompt": "what happens?", "video": "clip.mp4", "video_max_frames": 4})).await;
    assert_eq!(r["isError"], false, "{r}");
    let session = r["structuredContent"]["session"].as_str().unwrap().to_string();
    let c = core.conversation(&session).unwrap();
    let user = c.messages.iter().find(|m| m.role == bragi_core::Role::User).unwrap();
    assert_eq!(user.attachments.len(), 1);
    match &user.attachments[0].media {
        Some(bragi_core::Media::Video(v)) => assert_eq!(v.frames.len(), 4),
        other => panic!("not a video: {other:?}"),
    }
    let r = call(&rpc, "bragi_chat", json!({"prompt": "x", "video": "data:video/mp4;base64,!!!"})).await;
    assert_eq!(r["isError"], true, "{r}");
}

/// Native video (V4): `bragi_chat` with `video` sends the clip whole to a model that reads video
/// when it fits the limits (here the user's), and the frames otherwise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_sends_the_whole_clip_to_a_model_that_reads_video() {
    let (_tmp, _node, core, root) = setup().await;
    let rpc = Arc::new(bragi_mcp::server(tools(&core, &root)));
    let model = bragi_core::demo::WARM_MODEL;
    assert!(core.pool_model_reads_video(model));
    let base = MediaLimits::new().with_image_tokens(256).with_max_images(6);
    core.set_video_override(model, ModelLimits::new(None, base.with_video_max_seconds(60).with_video_max_bytes(100_000))).unwrap();
    let r = call(&rpc, "bragi_chat", json!({"prompt": "what happens?", "video": "clip.mp4", "model": model})).await;
    assert_eq!(r["isError"], false, "{r}");
    let session = r["structuredContent"]["session"].as_str().unwrap().to_string();
    let c = core.conversation(&session).unwrap();
    let answer = c.messages.iter().rev().find(|m| m.role == bragi_core::Role::Assistant).unwrap();
    assert!(answer.text.starts_with("I can see the whole clip (video/mp4, 36 bytes)."), "{}", answer.text);
    let user = c.messages.iter().find(|m| m.role == bragi_core::Role::User).unwrap();
    assert_eq!(user.attachments[0].label(), "clip.mp4 (video, whole clip (36 bytes))");

    // Over the model's length (the fake clip is 42 s): the frames.
    core.set_video_override(model, ModelLimits::new(None, base.with_video_max_seconds(30).with_video_max_bytes(100_000))).unwrap();
    let r = call(&rpc, "bragi_chat", json!({"prompt": "what happens?", "video": "clip.mp4", "model": model})).await;
    let session = r["structuredContent"]["session"].as_str().unwrap().to_string();
    let c = core.conversation(&session).unwrap();
    let answer = c.messages.iter().rev().find(|m| m.role == bragi_core::Role::Assistant).unwrap();
    assert!(answer.text.starts_with("I can see 6 image(s)"), "{}", answer.text);
}

/// The frames through a real MCP client over an in-memory stdio pipe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_mcp_client_reads_frames_as_images_with_their_times() {
    let (_tmp, _node, core, root) = setup().await;
    let server = Arc::new(bragi_mcp::server(tools(&core, &root)));
    let big: bragi_mcp::video::VideoSourceFn =
        Arc::new(|_p: &Path| Ok(Box::new(BigFrames(FakeSource::new(42_000, false))) as Box<dyn FrameSource + Send>));
    let big_server = Arc::new(bragi_mcp::server(tools_with(&core, &root, big)));
    let connect = |server: Arc<Rpc>, max_line: usize| async move {
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        let (sr, sw) = tokio::io::split(server_io);
        tokio::spawn(bragi_mcp::serve_stdio(server, sr, sw));
        let (cr, cw) = tokio::io::split(client_io);
        let options = ClientOptions { timeout: Duration::from_secs(5), ..ClientOptions::default() };
        Client::connect(StdioTransport::from_io_limited(cr, cw, max_line), options).await.unwrap()
    };

    let client = connect(server.clone(), bragi_core::mcp_servers::MAX_STDIO_LINE).await;
    let r = client.call_tool_content("bragi_video_frames", json!({"file": "clip.mp4", "count": 4})).await.unwrap();
    assert!(!r.is_error);
    let images: Vec<&Content> = r.content.iter().filter(|c| c.is_image()).collect();
    assert_eq!(images.len(), 4);
    let bytes = images[0].decoded_bytes().unwrap();
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "a JPEG");
    assert!(String::from_utf8_lossy(&bytes).ends_with("@1920x1080"));
    assert_eq!(images[0].mime_type(), Some("image/jpeg"));
    // Each image follows its time label.
    let labels: Vec<&str> = r.content.windows(2).filter(|w| w[1].is_image()).filter_map(|w| w[0].as_text()).collect();
    assert_eq!(labels, ["[00:05.2]", "[00:15.7]", "[00:26.2]", "[00:36.7]"]);
    let times: Vec<u64> = r.structured.as_ref().unwrap()["frames"].as_array().unwrap().iter().map(|f| f["at_ms"].as_u64().unwrap()).collect();
    assert_eq!(times, [5_250, 15_750, 26_250, 36_750]);
    // A block round-trips through its JSON.
    let again = Content::from_json(&images[1].to_json()).unwrap();
    assert_eq!(&again, images[1]);
    assert!(Content::from_json(&json!({"type": "image", "mimeType": "image/png"})).is_none(), "an image without data is not content");

    // 4 frames of 64 KiB: past a 128 KiB line cap the client drops the line unread and the call
    // times out; with Bragi's 64 MiB cap (`spawn_limited` for stdio servers) it arrives whole.
    let capped = connect(big_server.clone(), 128 * 1024).await;
    assert!(capped.call_tool_content("bragi_video_frames", json!({"file": "clip.mp4", "count": 4})).await.is_err());
    let roomy = connect(big_server, bragi_core::mcp_servers::MAX_STDIO_LINE).await;
    let r = roomy.call_tool_content("bragi_video_frames", json!({"file": "clip.mp4", "count": 4})).await.unwrap();
    assert!(r.content.iter().filter(|c| c.is_image()).all(|c| c.decoded_bytes().unwrap().len() == 64 * 1024));
}
