mod hf_hub;

pub mod anime_text;
pub mod aot_inpainting;
pub mod comic_text_bubble_detector;
pub mod comic_text_detector;
pub mod flux2_klein;
pub mod font_detector;
pub mod inpainting;
pub mod lama;
pub mod loading;
pub mod manga_ocr;
pub mod manga_text_segmentation_2025;
pub mod mit48px_ocr;
mod ops;
pub mod paddleocr_vl;
pub mod pp_doclayout_v3;
pub mod probability_map;
pub mod speech_bubble_segmentation;
pub mod types;

pub use types::{FontPrediction, NamedFontPrediction, Quad, TextDirection, TextRegion, TopFont};

use anyhow::Result;
use candle_core::utils::{cuda_is_available, metal_is_available};

pub use candle_core::Device;

static GPU_SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeDeviceOverview {
    pub summary: String,
    pub detail: String,
}

pub fn device(cpu: bool) -> Result<Device> {
    if cpu {
        Ok(Device::Cpu)
    } else if cuda_is_available()
        && *GPU_SUPPORTED.get_or_init(koharu_runtime::check_cuda_driver_support)
    {
        Ok(Device::new_cuda(0)?)
    } else if metal_is_available() {
        Ok(Device::new_metal(0)?)
    } else {
        tracing::warn!(
            "No GPU support detected; falling back to CPU. For better performance, ensure you have a compatible NVIDIA GPU with the latest drivers, or a recent Apple device with Metal support."
        );
        Ok(Device::Cpu)
    }
}

pub fn compute_device_overview(cpu: bool) -> ComputeDeviceOverview {
    let cuda_available = cuda_is_available();
    let metal_available = metal_is_available();
    let cuda_driver = koharu_runtime::nvidia_driver_version()
        .map(|version| version.to_string())
        .map_err(|err| format!("{err:#}"));
    let cuda_compute = koharu_runtime::compute_capability()
        .map(|(major, minor)| format!("{major}.{minor}"))
        .map_err(|err| format!("{err:#}"));
    let selected = match device(cpu) {
        Ok(device) => device_label(&device).to_string(),
        Err(err) => format!("probe failed: {err:#}"),
    };

    let cuda = if cuda_available {
        "available"
    } else {
        "unavailable"
    };
    let metal = if metal_available {
        "available"
    } else {
        "unavailable"
    };
    let driver = cuda_driver
        .as_ref()
        .map_or("unknown".to_string(), |version| version.to_string());
    let compute = cuda_compute
        .as_ref()
        .map_or("unknown".to_string(), |capability| capability.to_string());

    let detail = format!(
        "candle cuda={cuda}, cuda driver={driver}, cuda compute={compute}, metal={metal}{}{}",
        cuda_driver
            .err()
            .map(|err| format!("; cuda driver error: {err}"))
            .unwrap_or_default(),
        cuda_compute
            .err()
            .map(|err| format!("; cuda compute error: {err}"))
            .unwrap_or_default()
    );

    ComputeDeviceOverview {
        summary: format!(
            "ML device={selected}, cpu-only={}, cuda={cuda}, metal={metal}",
            if cpu { "true" } else { "false" }
        ),
        detail,
    }
}

fn device_label(device: &Device) -> &'static str {
    if device.is_cuda() {
        "cuda:0"
    } else if device.is_metal() {
        "metal:0"
    } else {
        "cpu"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_device_overview_reports_cpu_override() {
        let overview = compute_device_overview(true);

        assert!(overview.summary.contains("ML device=cpu"));
        assert!(overview.summary.contains("cpu-only=true"));
        assert!(overview.detail.contains("candle cuda="));
    }
}
