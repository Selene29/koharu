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
    pub selected_device: String,
    pub fallback_reason: Option<String>,
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
        .map(|(major, minor)| (major, minor))
        .map_err(|err| format!("{err:#}"));
    let selected = match device(cpu) {
        Ok(device) => device_label(&device).to_string(),
        Err(err) => format!("probe failed: {err:#}"),
    };
    let fallback_reason = ml_fallback_reason(
        cpu,
        &selected,
        cuda_available,
        metal_available,
        &cuda_compute,
    );

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
    let gpu_capability = cuda_compute
        .as_ref()
        .map_or("unknown".to_string(), |(major, minor)| {
            format!("{major}.{minor}")
        });

    let detail = format!(
        "candle cuda runtime={cuda}, nvidia driver CUDA={driver}, GPU compute capability={gpu_capability}, metal={metal}{}{}{}",
        cuda_driver
            .err()
            .map(|err| format!("; cuda driver error: {err}"))
            .unwrap_or_default(),
        cuda_compute
            .err()
            .map(|err| format!("; GPU compute capability error: {err}"))
            .unwrap_or_default(),
        fallback_reason
            .as_ref()
            .map(|reason| format!("; ML fallback: {reason}"))
            .unwrap_or_default()
    );
    let reason_suffix = fallback_reason
        .as_ref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();

    ComputeDeviceOverview {
        selected_device: selected.clone(),
        fallback_reason,
        summary: format!(
            "ML device={selected}{reason_suffix}, cpu-only={}, cuda={cuda}, metal={metal}",
            if cpu { "true" } else { "false" }
        ),
        detail,
    }
}

fn ml_fallback_reason(
    cpu: bool,
    selected: &str,
    cuda_available: bool,
    metal_available: bool,
    cuda_compute: &Result<(i32, i32), String>,
) -> Option<String> {
    if selected != "cpu" {
        return None;
    }
    if cpu {
        return Some("CPU-only override enabled".to_string());
    }
    if cuda_available {
        return match cuda_compute {
            Ok((major, minor)) if (*major, *minor) < (8, 0) => Some(format!(
                "GPU compute capability {major}.{minor} is below required 8.0 for ML CUDA"
            )),
            Ok((major, minor)) => Some(format!(
                "GPU compute capability {major}.{minor} was available but ML CUDA did not initialize"
            )),
            Err(err) => Some(format!(
                "GPU compute capability could not be queried: {err}"
            )),
        };
    }
    if metal_available {
        return Some("Metal was available but ML Metal did not initialize".to_string());
    }
    Some("no supported ML GPU backend detected".to_string())
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
        assert_eq!(overview.selected_device, "cpu");
        assert_eq!(
            overview.fallback_reason.as_deref(),
            Some("CPU-only override enabled")
        );
        assert!(overview.detail.contains("candle cuda runtime="));
    }

    #[test]
    fn ml_fallback_reason_distinguishes_gpu_capability_from_cuda_runtime() {
        let reason = ml_fallback_reason(false, "cpu", true, false, &Ok((7, 5)));

        assert_eq!(
            reason.as_deref(),
            Some("GPU compute capability 7.5 is below required 8.0 for ML CUDA")
        );
    }
}
