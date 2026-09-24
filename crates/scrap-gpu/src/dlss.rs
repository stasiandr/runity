//! NVIDIA's DLSS, through `dlss_wgpu`: the instance and the device made
//! with the Vulkan extensions it asks for, and its SDK, once per device.
//! The render module's upscaler is what uses it.
//!
//! Only on Windows and Linux, only on Vulkan, and only with the `dlss`
//! feature, which needs NVIDIA's DLSS SDK to build (`DLSS_SDK`, see
//! docs/stack.md). At run time the SDK's `libnvidia-ngx-dlss.so` or
//! `nvngx_dlss.dll` is looked for in `$DLSS_SDK/lib/...` and next to the
//! executable.

use std::sync::{Arc, Mutex};

pub use dlss_wgpu::super_resolution;
pub use dlss_wgpu::{DlssError, DlssFeatureFlags, DlssPerfQualityMode, DlssSdk, FeatureSupport};

use crate::GpuError;

// NGX's static library reads the registry (where the driver keeps its
// path), and `dlss_wgpu` does not link what that needs.
#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {}

/// scrap's application id for NVIDIA's NGX, which asks every application
/// for one (a UUID of its own choosing) to keep its models and logs under.
pub const PROJECT: uuid::Uuid = uuid::Uuid::from_u128(0x5b0c6b3e_3c1e_4f0a_9a57_72756e697479);

/// DLSS on this device.
pub struct Dlss {
    pub sdk: Arc<Mutex<DlssSdk>>,
}

impl Dlss {
    /// The SDK on `device`, if the driver has DLSS for it.
    pub(crate) fn new(device: &wgpu::Device, support: &FeatureSupport) -> Option<Self> {
        if !support.super_resolution_supported {
            tracing::info!("DLSS: the Vulkan driver lacks its extensions");
            return None;
        }
        match DlssSdk::new(PROJECT, device.clone()) {
            Ok(sdk) => Some(Self { sdk }),
            Err(e) => {
                // On Linux a driver older than the SDK refuses its library's
                // signature; `__NV_SIGNED_LOAD_CHECK=none` lets it load.
                tracing::warn!("DLSS unavailable: {e}");
                None
            }
        }
    }
}

/// A Vulkan instance with DLSS's extensions, and what the system has of it.
pub(crate) fn instance() -> Option<(wgpu::Instance, FeatureSupport)> {
    let mut support = FeatureSupport::default();
    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    match dlss_wgpu::create_instance(PROJECT, &descriptor, &mut support) {
        Ok(instance) if support.super_resolution_supported => Some((instance, support)),
        Ok(_) => None,
        Err(e) => {
            tracing::info!("DLSS: no Vulkan instance for it: {e}");
            None
        }
    }
}

/// The device `descriptor` asks for, with DLSS's extensions besides.
pub(crate) fn device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor,
    support: &mut FeatureSupport,
) -> Result<(wgpu::Device, wgpu::Queue), GpuError> {
    dlss_wgpu::request_device(PROJECT, adapter, descriptor, support, Some(descriptor.required_limits.clone()))
        .map_err(|e| GpuError::NoDevice(e.to_string()))
}
