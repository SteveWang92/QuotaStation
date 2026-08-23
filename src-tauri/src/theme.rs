//! Which palette every surface is drawn in.
//!
//! Two questions, and they have different answers. The dashboard, the quick panel and the
//! settings dialog follow the user's own choice — system, dark, or light. The taskbar
//! widget does not: it is a transparent window sitting inside the Windows taskbar, so it
//! has to match the taskbar it is drawn on or it becomes unreadable the moment those two
//! disagree. Windows keeps the two as separate registry values for exactly that reason,
//! and QuotaStation reads both.
//!
//! The core resolves the theme rather than leaving it to the renderer's
//! `prefers-color-scheme`: a WebView2 decides that from a window theme the application has
//! just forced, so a page asked to work it out for itself would answer with whatever it was
//! told rather than with what Windows is set to.

use serde::{Deserialize, Serialize};

/// What the user asked for.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the Windows app theme, and change with it while running.
    System,
    /// The default, because it is what QuotaStation looked like before there was a choice.
    #[default]
    Dark,
    Light,
}

/// What a surface is actually drawn in, once the preference has been resolved.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// The pair of answers every window needs, sent together so no surface has to ask twice.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSnapshot {
    /// For every window except the taskbar widget.
    pub app: Theme,
    /// For the taskbar widget alone.
    pub taskbar: Theme,
}

pub fn snapshot(preference: ThemePreference) -> ThemeSnapshot {
    ThemeSnapshot { app: resolve(preference), taskbar: taskbar_theme() }
}

pub fn resolve(preference: ThemePreference) -> Theme {
    match preference {
        ThemePreference::Dark => Theme::Dark,
        ThemePreference::Light => Theme::Light,
        // A machine that will not say is a machine QuotaStation leaves as it was.
        ThemePreference::System => personalize("AppsUseLightTheme")
            .map_or(Theme::Dark, |light| if light { Theme::Light } else { Theme::Dark }),
    }
}

/// The taskbar's own theme, which Windows tracks separately from the app theme.
pub fn taskbar_theme() -> Theme {
    personalize("SystemUsesLightTheme")
        .map_or(Theme::Dark, |light| if light { Theme::Light } else { Theme::Dark })
}

/// One of the two Personalize flags, or `None` when it cannot be read.
#[cfg(windows)]
fn personalize(value: &str) -> Option<bool> {
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        core::PCWSTR,
    };

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let key = wide(KEY);
    let name = wide(value);
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data != 0)
}

#[cfg(not(windows))]
fn personalize(_value: &str) -> Option<bool> {
    None
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value).encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_form_is_the_one_the_renderer_reads() {
        assert_eq!(serde_json::to_string(&ThemePreference::System).unwrap(), "\"system\"");
        assert_eq!(
            serde_json::to_string(&ThemeSnapshot { app: Theme::Light, taskbar: Theme::Dark })
                .unwrap(),
            "{\"app\":\"light\",\"taskbar\":\"dark\"}"
        );
    }
}
