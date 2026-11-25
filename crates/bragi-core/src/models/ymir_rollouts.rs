//! TEMPORARY: a minimal client for Ymir's `/ymir/rollouts/1` (Ymir `ymir_core::rollouts_net`,
//! Ymir 0.13.0), which reads and changes an inference node's versioned rollouts by that node's
//! rules. Ymir's crate is not published, so the few wire types Bragi sends and reads are mirrored
//! here: a read, adding a version, and giving it a share in percent (Ymir 0.15.0). It goes away with the Weirdnet hooks that
//! replace [`super::ymir_uploads`] (asked of the Weirdnet lead, 2026-11-25).

use serde::{Deserialize, Serialize};
use weirdnet_agent::remote::{Dialer, RemoteError};

/// Ymir's protocol id.
pub const ROLLOUTS_PROTOCOL: &str = "/ymir/rollouts/1";

/// The largest share Bragi asks for, in percent. A share named outright is not capped by Ymir's
/// canary rule (m3talsmith, 2026-11-25; Ymir 0.15.0).
pub const MAX_PERCENT: u32 = 100;

/// The shares the upload dialog offers as presets (any 0 to 100 can be typed).
pub const PRESETS: [u32; 7] = [0, 1, 5, 10, 20, 50, 100];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rollout {
    pub name: String,
    #[serde(default, rename = "version")]
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version: u32,
    pub model: String,
    #[serde(default)]
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Add {
        name: String,
        version: u32,
        model: String,
        weight: u32,
    },
    /// Exactly `percent` of new conversations; the other versions share the rest (Ymir 0.15.0).
    Share {
        name: String,
        version: u32,
        percent: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Change>,
    pub persist: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Rollouts {
    #[serde(default)]
    pub rollouts: Vec<Rollout>,
    #[serde(default)]
    pub said: Option<String>,
    #[serde(default)]
    pub saved: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Rollouts(Rollouts),
    Failed { message: String },
}

async fn request(dialer: &dyn Dialer, peer: &str, label: &str, req: &Request) -> Result<Rollouts, String> {
    let remote = |e: RemoteError| match e {
        RemoteError::Unsupported { .. } | RemoteError::Protocol { .. } => {
            format!("{label} runs a Ymir without {ROLLOUTS_PROTOCOL} (it needs Ymir 0.13.0 or later, with the inference role)")
        }
        e => format!("{label}: {e}"),
    };
    let mut ch = dialer.open(peer, ROLLOUTS_PROTOCOL).await.map_err(remote)?;
    ch.send(req).await.map_err(remote)?;
    match ch.recv::<Reply>().await {
        Ok(Some(Reply::Rollouts(r))) => Ok(r),
        Ok(Some(Reply::Failed { message })) => Err(format!("{label}: {message}")),
        Ok(None) => Err(remote(RemoteError::Closed)),
        Err(e) => Err(remote(e)),
    }
}

/// Adds `model` to the rollout `ask` names on its inference node (`label`) as the next version,
/// at weight 0, then gives it `ask.percent` percent of new conversations (at most
/// [`MAX_PERCENT`]); returns what the node said.
pub async fn add_version(
    dialer: &dyn Dialer,
    cert: Option<Vec<u8>>,
    label: &str,
    ask: &super::upload::RolloutAsk,
    model: &str,
    persist: bool,
) -> Result<String, String> {
    let (peer, name, percent) = (ask.inference.as_str(), ask.name.as_str(), ask.percent.min(MAX_PERCENT));
    let list = Request { cert: cert.clone(), change: None, persist };
    let now = request(dialer, peer, label, &list).await?;
    let next = now.rollouts.iter().filter(|r| r.name == name).flat_map(|r| r.versions.iter().map(|v| v.version)).max().unwrap_or(0) + 1;
    let change = Change::Add { name: name.to_string(), version: next, model: model.to_string(), weight: 0 };
    let mut got = request(dialer, peer, label, &Request { cert: cert.clone(), change: Some(change), persist }).await?;
    if percent > 0 {
        let share = Change::Share { name: name.to_string(), version: next, percent };
        got = request(dialer, peer, label, &Request { cert, change: Some(share), persist }).await.map_err(|e| {
            format!("{name}@{next} was added at weight 0, but its {percent}% share was not set ({e}; setting a percentage needs Ymir 0.15.0 there)")
        })?;
    }
    // Ymir words its reply for a terminal; the dialog shows it as one plain sentence.
    let said = got.said.unwrap_or_else(|| format!("Added {name}@{next}")).replace('`', "");
    let said = said.trim_end_matches('.');
    let where_ = match (got.saved, got.note) {
        (true, _) => format!("on {label}, saved in its node.toml"),
        (false, Some(n)) => format!("on {label}, live only: {n}"),
        (false, None) => format!("on {label}, live only until it restarts"),
    };
    Ok(format!("{said}, {where_}."))
}

/// The demo nodes' and tests' stand-in for Ymir's side: keeps rollouts in memory and adds
/// versions and shares (no eval gate: a share named outright has none).
pub(crate) async fn serve_stand_in(mut ch: weirdnet_agent::remote::Channel, state: &std::sync::Mutex<Vec<Rollout>>) {
    let Ok(Some(req)) = ch.recv::<Request>().await else { return };
    let reply = {
        let mut rs = state.lock().unwrap_or_else(|e| e.into_inner());
        match req.change {
            None => Reply::Rollouts(Rollouts { rollouts: rs.clone(), ..Default::default() }),
            Some(Change::Add { name, version, model, weight }) => {
                if !rs.iter().any(|r| r.name == name) {
                    rs.push(Rollout { name: name.clone(), versions: vec![] });
                }
                let r = rs.iter_mut().find(|r| r.name == name).expect("added");
                if r.versions.iter().any(|v| v.version == version) {
                    Reply::Failed { message: format!("{name}@{version} exists") }
                } else {
                    r.versions.push(Version { version, model: model.clone(), weight });
                    Reply::Rollouts(Rollouts {
                        rollouts: rs.clone(),
                        said: Some(format!("Added {name}@{version} (`{model}`) with weight {weight}.")),
                        saved: req.persist,
                        note: None,
                    })
                }
            }
            Some(Change::Share { name, version, percent }) => match rs.iter_mut().find(|r| r.name == name) {
                Some(r) if r.versions.iter().any(|v| v.version == version) => {
                    let others: u64 = r.versions.iter().filter(|v| v.version != version).map(|v| u64::from(v.weight)).sum();
                    let rest = u64::from(100 - percent.min(100));
                    for v in &mut r.versions {
                        v.weight = if v.version == version { percent } else { (rest * u64::from(v.weight)).checked_div(others).unwrap_or(0) as u32 };
                    }
                    Reply::Rollouts(Rollouts {
                        rollouts: rs.clone(),
                        said: Some(format!("{name}@{version} now gets {percent}.0% of new conversations.")),
                        saved: req.persist,
                        note: None,
                    })
                }
                _ => Reply::Failed { message: format!("no {name}@{version}") },
            },
        }
    };
    let _ = ch.send(&reply).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_matches_ymir() {
        let r = Request { cert: None, change: Some(Change::Add { name: "s".into(), version: 2, model: "m".into(), weight: 5 }), persist: true };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"change":{"op":"add","name":"s","version":2,"model":"m","weight":5},"persist":true}"#);
        let reply: Reply = serde_json::from_str(
            r#"{"reply":"rollouts","rollouts":[{"name":"s","version":[{"version":1,"model":"a","weight":100}]}],"said":"x","saved":true}"#,
        )
        .unwrap();
        let Reply::Rollouts(r) = reply else { panic!() };
        assert_eq!(r.rollouts[0].versions[0].weight, 100);
    }
}
