use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use tracing::info;

use crate::dxgi;

const VIRTUAL_DISPLAY_ID: &str = "root\\mttvdd";

/// Information about a single active display adapter.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Stable DeviceName, e.g. `\\.\DISPLAY1`.
    pub key: String,
    /// DXGI output index for ddagrab `output_idx=`.
    pub index: u32,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub is_virtual: bool,
}

impl std::fmt::Display for MonitorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = if self.is_virtual { "virtual" } else { "physical" };
        write!(
            f,
            "{} ({kind})  pos=({},{})  size={}x{}  idx={}",
            self.key, self.x, self.y, self.w, self.h, self.index
        )
    }
}

/// Live registry of connected displays. Thread-safe, refreshable on demand.
pub struct MonitorRegistry {
    monitors: Mutex<HashMap<String, MonitorInfo>>,
}

impl MonitorRegistry {
    pub fn new() -> Self {
        Self {
            monitors: Mutex::new(HashMap::new()),
        }
    }

    /// Re-enumerate all displays via Win32 + DXGI APIs.
    pub fn refresh(&self) -> Result<HashMap<String, MonitorInfo>> {
        let dxgi_map = dxgi::output_map();

        let mut found = HashMap::new();
        let mut enum_index: u32 = 0;

        #[cfg(windows)]
        {
            use windows::Win32::Graphics::Gdi::{
                DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
                EnumDisplayDevicesW, EnumDisplaySettingsW, ENUM_CURRENT_SETTINGS,
            };
            use windows::core::PCWSTR;

            let mut i: u32 = 0;
            loop {
                let mut adapter = DISPLAY_DEVICEW::default();
                adapter.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

                let ok =
                    unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut adapter, 0) };
                if !ok.as_bool() {
                    break;
                }

                let active = adapter.StateFlags.contains(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP);
                if active {
                    let dev_name = wchar_to_string(&adapter.DeviceName);
                    let dev_name_wide: Vec<u16> = adapter.DeviceName.iter().copied().collect();

                    let mut dm = DEVMODEW::default();
                    dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
                    let settings_ok = unsafe {
                        EnumDisplaySettingsW(
                            PCWSTR(dev_name_wide.as_ptr()),
                            ENUM_CURRENT_SETTINGS,
                            &mut dm,
                        )
                    };

                    if settings_ok.as_bool() {
                        let device_id = wchar_to_string(&adapter.DeviceID);
                        let is_virt = device_id.to_lowercase().contains(VIRTUAL_DISPLAY_ID);

                        let dxgi_idx = dxgi_map
                            .get(&dev_name)
                            .copied()
                            .unwrap_or(enum_index);

                        let info = MonitorInfo {
                            key: dev_name.clone(),
                            index: dxgi_idx,
                            x: unsafe { dm.Anonymous1.Anonymous2.dmPosition.x },
                            y: unsafe { dm.Anonymous1.Anonymous2.dmPosition.y },
                            w: dm.dmPelsWidth,
                            h: dm.dmPelsHeight,
                            is_virtual: is_virt,
                        };

                        if is_virt {
                            info!(
                                "virtual display identified: {} (DeviceID={})",
                                info.key, device_id
                            );
                        }
                        found.insert(dev_name, info);
                    }
                    enum_index += 1;
                }
                i += 1;
            }
        }

        info!(
            "monitor registry refreshed: {} display(s) — {}",
            found.len(),
            found.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        *self.monitors.lock().unwrap() = found.clone();
        Ok(found)
    }

    pub fn get_all(&self) -> HashMap<String, MonitorInfo> {
        self.monitors.lock().unwrap().clone()
    }

    /// Return the first virtual display (IDD driver), if any.
    pub fn get_virtual(&self) -> Option<MonitorInfo> {
        self.monitors
            .lock()
            .unwrap()
            .values()
            .find(|m| m.is_virtual)
            .cloned()
    }
}

/// Convert a null-terminated wchar array to a Rust String.
fn wchar_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
