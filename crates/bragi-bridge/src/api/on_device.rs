//! The optional on-device model (B4) for the Flutter app: off by default, a model downloaded
//! only after the user agrees to `on_device_consent`, then "On this device" in the picker.
//! Settings polls `on_device_status` while a download runs.

use flutter_rust_bridge::frb;

use bragi_core::{OnDeviceModel, OnDeviceStatus};

use super::bragi::{core, err};

/// What Settings shows about the on-device model.
#[derive(Debug, Clone)]
pub struct OnDeviceView {
    pub enabled: bool,
    pub model: Option<String>,
    pub model_label: Option<String>,
    pub installed: bool,
    /// The engine this build runs; `None` in a build without one.
    pub engine: Option<String>,
    /// The model downloading, with how far it is (0.0 to 1.0) and `12.3 MB of 2.84 GB`.
    pub downloading: Option<String>,
    pub progress: f64,
    pub progress_label: String,
    pub error: Option<String>,
    /// On, downloaded and runnable: it shows in the picker.
    pub ready: bool,
    pub line: String,
}

/// A model this device can download and run.
#[derive(Debug, Clone)]
pub struct OnDeviceModelView {
    pub id: String,
    pub label: String,
    pub size_label: String,
    pub installed: bool,
    /// Part of it downloaded (the next download resumes).
    pub partial: bool,
    pub chosen: bool,
}

pub(crate) fn status_view(s: &OnDeviceStatus) -> OnDeviceView {
    let (downloading, progress, progress_label) = match &s.download {
        Some(d) => (
            Some(d.id.clone()),
            if d.total == 0 { 0.0 } else { (d.done as f64 / d.total as f64).min(1.0) },
            format!("{} of {}", bragi_core::on_device::human(d.done), bragi_core::on_device::human(d.total)),
        ),
        None => (None, 0.0, String::new()),
    };
    OnDeviceView {
        enabled: s.enabled,
        model: s.model.clone(),
        model_label: s.model_label.clone(),
        installed: s.installed,
        engine: s.engine.clone(),
        downloading,
        progress,
        progress_label,
        error: s.error.clone(),
        ready: s.ready(),
        line: s.line(),
    }
}

fn model_view(m: OnDeviceModel) -> OnDeviceModelView {
    OnDeviceModelView { id: m.id, label: m.label, size_label: m.size_label, installed: m.installed, partial: m.partial > 0, chosen: m.chosen }
}

#[frb(sync)]
pub fn on_device_status() -> Result<OnDeviceView, String> {
    Ok(status_view(&core()?.state().on_device))
}

/// Turns the on-device model on or off (on downloads nothing).
#[frb(sync)]
pub fn set_on_device(enabled: bool) -> Result<OnDeviceView, String> {
    Ok(status_view(&core()?.set_on_device(enabled).map_err(err)?))
}

#[frb(sync)]
pub fn on_device_models() -> Result<Vec<OnDeviceModelView>, String> {
    Ok(core()?.on_device_models().into_iter().map(model_view).collect())
}

#[frb(sync)]
pub fn choose_on_device_model(id: String) -> Result<OnDeviceView, String> {
    let c = core()?;
    c.choose_on_device_model(&id).map_err(err)?;
    Ok(status_view(&c.state().on_device))
}

/// What the user agrees to before `id` downloads.
#[frb(sync)]
pub fn on_device_consent(id: String) -> Result<Vec<String>, String> {
    core()?.on_device_consent(&id).map_err(err)
}

/// Starts downloading `id`; `consent` is the user's yes to `on_device_consent` (nothing
/// downloads without it).
#[frb(sync)]
pub fn download_on_device_model(id: String, consent: bool) -> Result<OnDeviceView, String> {
    let c = core()?;
    c.download_on_device_model(&id, consent).map_err(err)?;
    Ok(status_view(&c.state().on_device))
}

#[frb(sync)]
pub fn cancel_on_device_download() -> bool {
    core().is_ok_and(|c| c.cancel_on_device_download())
}

#[frb(sync)]
pub fn remove_on_device_model(id: String) -> Result<OnDeviceView, String> {
    let c = core()?;
    c.remove_on_device_model(&id).map_err(err)?;
    Ok(status_view(&c.state().on_device))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bragi_core::OnDeviceDownload;

    #[test]
    fn a_download_reads_as_a_fraction_and_sizes() {
        let s = OnDeviceStatus {
            enabled: true,
            download: Some(OnDeviceDownload { id: "gemma-4-e2b".into(), done: 1_420_000_000, total: 2_840_000_000 }),
            ..OnDeviceStatus::default()
        };
        let v = status_view(&s);
        assert_eq!(v.downloading.as_deref(), Some("gemma-4-e2b"));
        assert!((v.progress - 0.5).abs() < 1e-9);
        assert_eq!(v.progress_label, "1.42 GB of 2.84 GB");
        assert!(!v.ready);
        assert_eq!(status_view(&OnDeviceStatus::default()).line, "Off. Nothing is downloaded or run on this device.");
    }
}
