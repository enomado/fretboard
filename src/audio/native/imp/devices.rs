use std::collections::HashSet;
use std::process::{
    Command as ProcessCommand,
    Stdio,
};

use cpal::traits::{
    DeviceTrait,
    HostTrait,
};
use cpal::{
    BufferSize,
    Device,
    SupportedBufferSize,
    SupportedStreamConfig,
};

use super::{
    CPAL_DEFAULT_OUTPUT_LOOPBACK_ID,
    CPAL_INPUT_ID_PREFIX,
    LOW_LATENCY_TARGET_FRAMES,
    PULSE_DEFAULT_MONITOR_ID,
    PULSE_DEFAULT_SOURCE_ID,
    PULSE_INPUT_ID_PREFIX,
};
use crate::audio::types::{
    AudioInputKind,
    AudioInputOption,
};

// Pulse/PipeWire routes are preferred on Linux: they track the live default
// source and avoid noisy ALSA compatibility duplicates. CPAL devices stay as a
// direct-device fallback.
pub(super) fn enumerate_input_options() -> Vec<AudioInputOption> {
    let pulse_available = pulse_input_available();
    let mut seen = HashSet::new();
    let mut options: Vec<AudioInputOption> = Vec::new();

    if pulse_available {
        push_unique_input(
            &mut options,
            &mut seen,
            AudioInputOption {
                id:    PULSE_DEFAULT_SOURCE_ID.to_owned(),
                label: "Mic • Default microphone (Pulse/PipeWire)".to_owned(),
                kind:  AudioInputKind::Microphone,
            },
        );

        for option in pulse_source_input_options() {
            push_unique_input(&mut options, &mut seen, option);
        }

        push_unique_input(
            &mut options,
            &mut seen,
            AudioInputOption {
                id:    PULSE_DEFAULT_MONITOR_ID.to_owned(),
                label: "System • Default monitor (Pulse/PipeWire)".to_owned(),
                kind:  AudioInputKind::System,
            },
        );
    }

    #[cfg(target_os = "windows")]
    if let Some(device) = cpal::default_host().default_output_device() {
        let name = cpal_device_display_name(&device);
        push_unique_input(
            &mut options,
            &mut seen,
            AudioInputOption {
                id:    CPAL_DEFAULT_OUTPUT_LOOPBACK_ID.to_owned(),
                label: format!("System • {name} (WASAPI loopback)"),
                kind:  AudioInputKind::System,
            },
        );
    }

    let host = cpal::default_host();
    let default_device = host.default_input_device();
    let default_name = default_device.as_ref().map(cpal_device_display_name);
    let default_id = default_device.as_ref().map(cpal_device_route_id);
    let Ok(devices) = host.input_devices() else {
        options.sort_by_key(input_option_sort_key);
        return options;
    };

    let mut entries: Vec<(String, String, AudioInputKind, bool)> = devices
        .filter_map(|device| {
            let id = cpal_device_route_id(&device);
            let name = cpal_device_display_name(&device);
            if pulse_available && is_cpal_audio_server_proxy(&id, &name) {
                return None;
            }
            let kind = classify_input_kind(&name, default_name.as_deref());
            let is_default = default_id.as_deref() == Some(id.as_str());
            Some((id, name, kind, is_default))
        })
        .collect();

    // Если ALSA не отметил ни одного устройства как Microphone —
    // помечаем им дефолтное (либо первое не-System), чтобы UI его
    // не прятал.
    if !entries
        .iter()
        .any(|(_, _, kind, _)| *kind == AudioInputKind::Microphone)
    {
        let fallback = entries
            .iter()
            .position(|(_, _, kind, is_default)| *kind != AudioInputKind::System && *is_default)
            .or_else(|| {
                entries
                    .iter()
                    .position(|(_, _, kind, _)| *kind != AudioInputKind::System)
            });
        if let Some(i) = fallback {
            entries[i].2 = AudioInputKind::Microphone;
        }
    }

    for (id, name, kind, is_default) in entries {
        push_unique_input(
            &mut options,
            &mut seen,
            AudioInputOption {
                id,
                label: format_input_label(&name, kind, is_default),
                kind,
            },
        );
    }

    options.sort_by_key(input_option_sort_key);
    options
}

fn push_unique_input(
    options: &mut Vec<AudioInputOption>,
    seen: &mut HashSet<String>,
    option: AudioInputOption,
) {
    if seen.insert(option.id.clone()) {
        options.push(option);
    }
}

fn pulse_source_input_options() -> Vec<AudioInputOption> {
    let Ok(output) = ProcessCommand::new("pactl").args(["list", "sources"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_pulse_source_input_options(&text)
}

pub(super) fn parse_pulse_source_input_options(text: &str) -> Vec<AudioInputOption> {
    let mut options = Vec::new();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Source #") {
            push_pulse_source_option(&mut options, name.take(), description.take());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Name:") {
            name = Some(value.trim().to_owned());
        } else if let Some(value) = trimmed.strip_prefix("Description:") {
            description = Some(value.trim().to_owned());
        }
    }
    push_pulse_source_option(&mut options, name, description);
    options
}

fn push_pulse_source_option(
    options: &mut Vec<AudioInputOption>,
    name: Option<String>,
    description: Option<String>,
) {
    let Some(name) = name else {
        return;
    };
    if name == "@DEFAULT_SOURCE@" || name == "@DEFAULT_MONITOR@" {
        return;
    }
    let label_name = description
        .filter(|desc| !desc.trim().is_empty())
        .unwrap_or_else(|| name.clone());
    let kind = classify_input_kind(&format!("{name} {label_name}"), None);
    options.push(AudioInputOption {
        id: format!("{PULSE_INPUT_ID_PREFIX}{name}"),
        label: format_input_label(&label_name, kind, false),
        kind,
    });
}

fn input_option_sort_key(option: &AudioInputOption) -> (u8, u8, String) {
    let kind_rank = match option.kind {
        AudioInputKind::Microphone => 0,
        AudioInputKind::System => 1,
        AudioInputKind::Other => 2,
    };
    let route_rank = if option.id == PULSE_DEFAULT_SOURCE_ID || option.id == PULSE_DEFAULT_MONITOR_ID {
        0
    } else if option.id.starts_with(PULSE_INPUT_ID_PREFIX) {
        1
    } else {
        2
    };
    (kind_rank, route_rank, option.label.to_lowercase())
}

pub(super) struct SelectedCpalCapture {
    pub device:      Device,
    pub selected_id: String,
    pub config:      SupportedStreamConfig,
}

pub(super) fn select_cpal_capture(
    host: &cpal::Host,
    requested: Option<&str>,
) -> Result<SelectedCpalCapture, String> {
    #[cfg(target_os = "windows")]
    if requested == Some(CPAL_DEFAULT_OUTPUT_LOOPBACK_ID) {
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device found for WASAPI loopback".to_owned())?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("Output loopback config error: {e}"))?;
        return Ok(SelectedCpalCapture {
            device,
            selected_id: CPAL_DEFAULT_OUTPUT_LOOPBACK_ID.to_owned(),
            config,
        });
    }

    let device = select_input_device(host, requested)?;
    let selected_id = cpal_device_route_id(&device);
    let config = device
        .default_input_config()
        .map_err(|e| format!("Input config error: {e}"))?;
    Ok(SelectedCpalCapture {
        device,
        selected_id,
        config,
    })
}

fn select_input_device(host: &cpal::Host, requested: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(requested) = requested {
        if let Some(device_id) = parse_cpal_device_id(requested)
            && let Some(device) = host.device_by_id(&device_id)
        {
            return Ok(device);
        }
        let devices = host
            .input_devices()
            .map_err(|e| format!("Failed to enumerate input devices: {e}"))?;
        for device in devices {
            let device_name = cpal_device_display_name(&device);
            let device_id = cpal_device_route_id(&device);
            if device_id == requested || device_name == requested {
                return Ok(device);
            }
        }
        return Err(format!("Input device not found: {requested}"));
    }
    host.default_input_device()
        .ok_or_else(|| "No input device found".to_owned())
}

fn classify_input_kind(name: &str, default_name: Option<&str>) -> AudioInputKind {
    let lowered = name.to_lowercase();
    let system_markers = [
        "monitor",
        "loopback",
        "stereo mix",
        "what u hear",
        "blackhole",
        "soundflower",
    ];
    let microphone_markers = [
        "alsa_input",
        "microphone",
        " mic",
        "mono-fallback",
        "multichannel-input",
        "capture",
    ];
    let looks_like_microphone =
        microphone_markers.iter().any(|m| lowered.contains(m)) || default_name == Some(name);

    if system_markers.iter().any(|m| lowered.contains(m)) {
        AudioInputKind::System
    } else if looks_like_microphone {
        AudioInputKind::Microphone
    } else {
        AudioInputKind::Other
    }
}

fn format_input_label(name: &str, kind: AudioInputKind, is_default: bool) -> String {
    let tag = match kind {
        AudioInputKind::Microphone => "Mic",
        AudioInputKind::System => "System",
        AudioInputKind::Other => "Input",
    };
    if is_default {
        format!("{tag} • {name} (Default)")
    } else {
        format!("{tag} • {name}")
    }
}

pub(super) fn cpal_device_display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|desc| desc.name().to_owned())
        .unwrap_or_else(|_| "Unknown input".to_owned())
}

pub(super) fn cpal_device_route_id(device: &cpal::Device) -> String {
    match device.id() {
        Ok(id) => format!("{CPAL_INPUT_ID_PREFIX}{id}"),
        Err(_) => {
            format!(
                "{CPAL_INPUT_ID_PREFIX}compat::{}",
                cpal_device_display_name(device)
            )
        }
    }
}

fn parse_cpal_device_id(requested: &str) -> Option<cpal::DeviceId> {
    requested
        .strip_prefix(CPAL_INPUT_ID_PREFIX)?
        .parse::<cpal::DeviceId>()
        .ok()
}

pub(super) fn preferred_low_latency_buffer(range: &SupportedBufferSize) -> BufferSize {
    match range {
        SupportedBufferSize::Range { min, max } => {
            let requested = LOW_LATENCY_TARGET_FRAMES.clamp(*min, *max);
            BufferSize::Fixed(requested)
        }
        SupportedBufferSize::Unknown => BufferSize::Default,
    }
}

pub(super) fn low_latency_monitor_ring_len(sample_rate: u32) -> usize {
    ((sample_rate as usize) * 3 / 100).max(256)
}

fn is_cpal_audio_server_proxy(id: &str, name: &str) -> bool {
    let lowered = format!("{} {}", id.to_lowercase(), name.to_lowercase());
    lowered.contains("compat::default")
        || lowered.contains("compat::pulse")
        || lowered.contains("compat::pipewire")
        || lowered == format!("{CPAL_INPUT_ID_PREFIX}default default")
        || lowered == format!("{CPAL_INPUT_ID_PREFIX}pulse pulse")
        || lowered == format!("{CPAL_INPUT_ID_PREFIX}pipewire pipewire")
}

fn pulse_input_available() -> bool {
    ProcessCommand::new("parec")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::parse_pulse_source_input_options;
    use crate::audio::AudioInputKind;

    #[test]
    fn pulse_source_parser_keeps_mics_and_monitors_classified() {
        let sources = parse_pulse_source_input_options(
            r#"
Source #42
    State: RUNNING
    Name: alsa_input.usb-Focusrite_Scarlett_Solo-00.mono-fallback
    Description: Scarlett Solo Analog Mono
Source #43
    State: IDLE
    Name: alsa_output.pci-0000_00_1f.3.analog-stereo.monitor
    Description: Built-in Audio Analog Stereo Monitor
"#,
        );

        assert_eq!(sources.len(), 2);
        assert_eq!(
            sources[0].id,
            "pulse::alsa_input.usb-Focusrite_Scarlett_Solo-00.mono-fallback"
        );
        assert_eq!(sources[0].kind, AudioInputKind::Microphone);
        assert!(sources[0].label.contains("Scarlett Solo"));
        assert_eq!(sources[1].kind, AudioInputKind::System);
    }

    #[test]
    fn pulse_source_parser_ignores_virtual_default_aliases() {
        let sources = parse_pulse_source_input_options(
            r#"
Source #1
    Name: @DEFAULT_SOURCE@
    Description: Default Source
Source #2
    Name: @DEFAULT_MONITOR@
    Description: Default Monitor
"#,
        );

        assert!(sources.is_empty());
    }
}
