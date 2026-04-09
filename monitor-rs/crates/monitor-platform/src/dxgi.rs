use std::collections::HashMap;
use tracing::{info, warn};

/// Walk IDXGIFactory -> IDXGIAdapter -> IDXGIOutput and read each output's
/// DeviceName from `DXGI_OUTPUT_DESC`. Returns `{DeviceName: global_output_idx}`
/// where `global_output_idx` is the value ddagrab's `output_idx=` expects.
///
/// Returns an empty map on failure so callers fall back to EnumDisplayDevices order.
pub fn output_map() -> HashMap<String, u32> {
    #[cfg(windows)]
    {
        output_map_windows()
    }
    #[cfg(not(windows))]
    {
        HashMap::new()
    }
}

#[cfg(windows)]
fn output_map_windows() -> HashMap<String, u32> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory, IDXGIFactory, IDXGIOutput};

    let factory: IDXGIFactory = match unsafe { CreateDXGIFactory() } {
        Ok(f) => f,
        Err(e) => {
            warn!("dxgi::output_map: CreateDXGIFactory failed: {e}");
            return HashMap::new();
        }
    };

    let mut result = HashMap::new();
    let mut global_idx: u32 = 0;
    let mut adapter_i: u32 = 0;

    loop {
        let adapter = match unsafe { factory.EnumAdapters(adapter_i) } {
            Ok(a) => a,
            Err(_) => break,
        };

        let mut output_i: u32 = 0;
        loop {
            let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(output_i) } {
                Ok(o) => o,
                Err(_) => break,
            };

            match unsafe { output.GetDesc() } {
                Ok(desc) => {
                    let name = wchar_to_string(&desc.DeviceName);
                    result.insert(name, global_idx);
                }
                Err(e) => {
                    warn!("dxgi::output_map: GetDesc failed for output {global_idx}: {e}");
                }
            }
            global_idx += 1;
            output_i += 1;
        }
        adapter_i += 1;
    }

    info!(
        "dxgi::output_map: {}",
        if result.is_empty() {
            "(empty)".to_string()
        } else {
            result
                .iter()
                .map(|(k, v)| format!("{k}→{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    result
}

fn wchar_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
