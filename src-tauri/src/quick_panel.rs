//! The panel a click on the tray icon or the taskbar status opens: how large it is, where
//! it goes, and how it grows once the renderer has measured its own content.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tauri::{Manager, PhysicalPosition};

use crate::{APP, AppState, log, settings, taskbar};

/// The panel shows one column per provider, so its width follows how many are enabled.
/// Sizing it as it opens keeps the edge anchoring below working from the real size.
///
/// This is the width the columns are laid out at, so a scaled display is given the scaled
/// window; an unscaled width would squeeze the column and reflow the panel taller than the
/// height the renderer measured.
const QUICK_PANEL_COLUMN_WIDTH: f64 = 390.0;
/// The compact density stacks its providers, so the one column holds all of them and the
/// window is this wide whatever is enabled.
const QUICK_PANEL_COMPACT_WIDTH: f64 = 180.0;
/// Only what the window opens at before the renderer has measured anything. Every height
/// after the first render comes from [`set_quick_panel_height`].
const QUICK_PANEL_HEIGHT: u32 = 730;
/// The gap the panel keeps from every edge of the work area, shared by the placement and
/// the growth below so a panel that grows stops exactly where one that opens would.
const QUICK_PANEL_MARGIN: f64 = 12.0;

/// What the window rect holds that the page is not drawn in.
///
/// An undecorated window with a shadow keeps an invisible resize frame, so its window rect
/// is larger than its content area — 18 x 10 physical pixels at 125% here. `outer_size`,
/// `outer_position` and the work area are all in window-rect coordinates while `set_size`
/// takes a content size, so every placement below stays in window-rect units and converts
/// exactly once, in [`resize_quick_panel`]. Mixing the two would grow the window by this
/// frame on every open and leave a band of bare background around the card.
fn quick_panel_frame(panel: &tauri::WebviewWindow) -> tauri::PhysicalSize<u32> {
    let Ok(outer) = panel.outer_size() else { return tauri::PhysicalSize::new(0, 0) };
    let inner = panel.inner_size().unwrap_or(outer);
    tauri::PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    )
}

pub(crate) fn resize_quick_panel(
    panel: &tauri::WebviewWindow,
    frame: tauri::PhysicalSize<u32>,
    outer: tauri::PhysicalSize<u32>,
) {
    let _ = panel.set_size(tauri::PhysicalSize::new(
        outer.width.saturating_sub(frame.width).max(1),
        outer.height.saturating_sub(frame.height).max(1),
    ));
}

/// The window rect the panel needs for `providers` columns, given the height it already holds.
fn quick_panel_size(
    density: settings::QuickPanelDensity,
    providers: usize,
    height: u32,
    scale_factor: f64,
    frame: tauri::PhysicalSize<u32>,
) -> tauri::PhysicalSize<u32> {
    let layout = match density {
        settings::QuickPanelDensity::Standard => {
            QUICK_PANEL_COLUMN_WIDTH * providers.clamp(1, 2) as f64
        }
        settings::QuickPanelDensity::Compact => QUICK_PANEL_COMPACT_WIDTH,
    };
    let columns = layout * scale_factor.max(1.0);
    tauri::PhysicalSize::new((columns.round() as u32).saturating_add(frame.width), height)
}

/// Where the panel sits once the renderer reports a different content height.
///
/// The bottom edge is the fixed one: the placement anchored it beside the tray, so the
/// panel grows away from that edge rather than sliding out from under the pointer. Content
/// taller than the work area is clamped to it, and the panel scrolls its own contents from
/// there — there is nowhere left to grow.
fn quick_panel_growth(
    work_area: tauri::PhysicalRect<i32, u32>,
    position: PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    requested_height: u32,
) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let margin = QUICK_PANEL_MARGIN as i32;
    let available = (work_area.size.height as f64 - QUICK_PANEL_MARGIN * 2.0).max(1.0) as u32;
    let height = requested_height.clamp(1, available);
    let top_limit = work_area.position.y + margin;
    let bottom_limit = work_area.position.y + work_area.size.height as i32 - margin;
    let bottom = (position.y + size.height as i32).min(bottom_limit);
    let y = (bottom - height as i32).max(top_limit);
    (PhysicalPosition::new(position.x, y), tauri::PhysicalSize::new(size.width, height))
}

/// The height the renderer measured, in CSS pixels, for a window that has no frame to
/// trim it to its contents.
#[tauri::command]
pub(crate) fn set_quick_panel_height(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    let Some(panel) = app.get_webview_window("quick-panel") else { return Ok(()) };
    if !height.is_finite() || height <= 0.0 {
        return Ok(());
    }
    let scale_factor = panel.scale_factor().map_err(|error| error.to_string())?;
    let frame = quick_panel_frame(&panel);
    let size = panel.outer_size().map_err(|error| error.to_string())?;
    let position = panel.outer_position().map_err(|error| error.to_string())?;
    let requested = ((height * scale_factor).round().clamp(1.0, u32::MAX as f64) as u32)
        .saturating_add(frame.height);
    let work_area = panel.current_monitor().ok().flatten().map(|monitor| *monitor.work_area());
    let (next_position, next_size) = match work_area {
        Some(work_area) => quick_panel_growth(work_area, position, size, requested),
        // Without a monitor there is nothing to clamp against, so the request stands and
        // the bottom edge still holds.
        None => (
            PhysicalPosition::new(position.x, position.y + size.height as i32 - requested as i32),
            tauri::PhysicalSize::new(size.width, requested),
        ),
    };
    if next_size == size && next_position == position {
        return Ok(());
    }
    resize_quick_panel(&panel, frame, next_size);
    panel.set_position(next_position).map_err(|error| error.to_string())?;
    Ok(())
}

fn quick_panel_placement(
    work_area: tauri::PhysicalRect<i32, u32>,
    tray_position: PhysicalPosition<f64>,
    tray_size: tauri::PhysicalSize<f64>,
    requested_size: tauri::PhysicalSize<u32>,
) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let margin = QUICK_PANEL_MARGIN;
    let left = work_area.position.x as f64;
    let top = work_area.position.y as f64;
    let right = left + work_area.size.width as f64;
    let bottom = top + work_area.size.height as f64;
    let available_width = (work_area.size.width as f64 - margin * 2.0).max(1.0);
    let available_height = (work_area.size.height as f64 - margin * 2.0).max(1.0);
    let panel_width = (requested_size.width as f64).min(available_width);
    let panel_height = (requested_size.height as f64).min(available_height);
    let panel_size =
        tauri::PhysicalSize::new(panel_width.round() as u32, panel_height.round() as u32);
    let anchor = PhysicalPosition::new(
        tray_position.x + tray_size.width / 2.0,
        tray_position.y + tray_size.height / 2.0,
    );
    let nearest = [
        (anchor.x - left, "left"),
        (right - anchor.x, "right"),
        (anchor.y - top, "top"),
        (bottom - anchor.y, "bottom"),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, edge)| edge)
    .unwrap_or("bottom");
    let max_x = (right - panel_width - margin).max(left + margin);
    let max_y = (bottom - panel_height - margin).max(top + margin);
    let clamp_x = |value: f64| value.clamp(left + margin, max_x);
    let clamp_y = |value: f64| value.clamp(top + margin, max_y);
    let (x, y) = match nearest {
        "top" => (
            clamp_x(tray_position.x + tray_size.width - panel_width),
            clamp_y(tray_position.y + tray_size.height + margin),
        ),
        "left" => (
            clamp_x(tray_position.x + tray_size.width + margin),
            clamp_y(tray_position.y + tray_size.height - panel_height),
        ),
        "right" => (
            clamp_x(tray_position.x - panel_width - margin),
            clamp_y(tray_position.y + tray_size.height - panel_height),
        ),
        _ => (
            clamp_x(tray_position.x + tray_size.width - panel_width),
            clamp_y(tray_position.y - panel_height - margin),
        ),
    };
    (PhysicalPosition::new(x.round() as i32, y.round() as i32), panel_size)
}

/// Shows or hides the panel beside `anchor`, given in physical screen coordinates: the tray
/// icon for a click on the tray, the docked widget for a click on the taskbar status. Both
/// open the one panel — a second panel for the second surface would be the same readings
/// drawn twice.
///
/// Reports whether the panel is now open.
pub(crate) fn toggle_quick_panel_beside(
    app: &tauri::AppHandle,
    anchor_position: PhysicalPosition<f64>,
    anchor_size: tauri::PhysicalSize<f64>,
) -> bool {
    let Some(panel) = app.get_webview_window("quick-panel") else { return false };
    let state = app.state::<Arc<AppState>>();
    // One click opens the panel once. The tray icon and the taskbar status sit in the same
    // corner and there is one panel between them, so a second request arriving on the heels
    // of the first is the same click reaching a second path — obeying it moved the panel to
    // the other anchor, which read as a second window replacing the first.
    if let Ok(mut timing) = state.quick_panel.lock() {
        if timing.toggled_at.is_some_and(|at| at.elapsed() < Duration::from_millis(300)) {
            return panel.is_visible().unwrap_or(false);
        }
        timing.toggled_at = Some(Instant::now());
    }
    // The renderer has already sized the window to its contents, so the panel opens at the
    // height it currently holds rather than at the height it was configured with.
    let frame = quick_panel_frame(&panel);
    let current_height = panel.outer_size().map(|size| size.height).unwrap_or(QUICK_PANEL_HEIGHT);
    let requested_size = quick_panel_size(
        state.settings().quick_panel_density,
        state.quota_column_count(),
        current_height,
        panel.scale_factor().unwrap_or(1.0),
        frame,
    );
    if let Ok(mut timing) = state.quick_panel.lock()
        && timing
            .focus_lost_at
            .is_some_and(|lost_at| lost_at.elapsed() < Duration::from_millis(500))
    {
        timing.focus_lost_at = None;
        return false;
    }
    if panel.is_visible().unwrap_or(false) {
        log::write("quick panel closed from the tray");
        let _ = panel.hide();
        return false;
    }
    log::write(format!(
        "quick panel opened beside the tray, {} column(s)",
        state.quota_column_count()
    ));

    let centre =
        (anchor_position.x + anchor_size.width / 2.0, anchor_position.y + anchor_size.height / 2.0);
    let monitor = app.monitor_from_point(centre.0, centre.1).ok().flatten();
    let (x, y) = if let Some(monitor) = monitor {
        let (position, fitted_size) = quick_panel_placement(
            *monitor.work_area(),
            anchor_position,
            anchor_size,
            requested_size,
        );
        resize_quick_panel(&panel, frame, fitted_size);
        (position.x as f64, position.y as f64)
    } else {
        resize_quick_panel(&panel, frame, requested_size);
        (
            anchor_position.x - requested_size.width as f64,
            anchor_position.y - requested_size.height as f64,
        )
    };
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    if let Ok(mut timing) = state.quick_panel.lock() {
        timing.shown_at = Some(Instant::now());
    }
    let _ = panel.show();
    let _ = panel.set_focus();
    true
}

/// The tray reports its icon in whichever unit the platform uses, so the click position —
/// which is already physical — is what identifies the monitor whose scale converts it.
pub(crate) fn toggle_quick_panel(
    app: &tauri::AppHandle,
    click: PhysicalPosition<f64>,
    tray_rect: tauri::Rect,
) {
    let scale_factor = app
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    toggle_quick_panel_beside(
        app,
        tray_rect.position.to_physical(scale_factor),
        tray_rect.size.to_physical(scale_factor),
    );
}

/// Opens the panel above the taskbar status, anchored to the widget rather than to the tray
/// icon.
///
/// Called from the click watch in [`taskbar`], which runs inside a low-level mouse hook, so
/// the work is queued onto the main thread rather than done there: a hook that takes its
/// time is a hook the system stops calling.
pub(crate) fn open_quick_panel_from_taskbar() {
    let Some(app) = APP.get().cloned() else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Ok((position, size)) = taskbar::widget_screen_rect(&handle) else { return };
        if toggle_quick_panel_beside(&handle, position, size) {
            let _ = taskbar::raise_window(&handle, "quick-panel");
        }
    });
}

#[cfg(test)]
mod quick_panel_tests {
    use super::*;

    fn placement(
        width: u32,
        height: u32,
        tray_x: f64,
        tray_y: f64,
    ) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
        quick_panel_placement(
            tauri::PhysicalRect {
                position: PhysicalPosition::new(0, 0),
                size: tauri::PhysicalSize::new(width, height),
            },
            PhysicalPosition::new(tray_x, tray_y),
            tauri::PhysicalSize::new(40.0, 40.0),
            tauri::PhysicalSize::new(780, 730),
        )
    }

    fn work_area(height: u32) -> tauri::PhysicalRect<i32, u32> {
        tauri::PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: tauri::PhysicalSize::new(1280, height),
        }
    }

    #[test]
    fn a_shorter_panel_keeps_its_bottom_edge_and_a_taller_one_grows_upwards() {
        let position = PhysicalPosition::new(400, 300);
        let size = tauri::PhysicalSize::new(390, 400);
        let bottom = position.y + size.height as i32;
        for requested in [200, 400, 620] {
            let (next_position, next_size) =
                quick_panel_growth(work_area(1000), position, size, requested);
            assert_eq!(next_size.height, requested);
            assert_eq!(next_size.width, size.width, "only the height follows the contents");
            assert_eq!(next_position.x, position.x);
            assert_eq!(next_position.y + next_size.height as i32, bottom);
        }
    }

    #[test]
    fn a_panel_taller_than_the_work_area_is_clamped_inside_it() {
        let (position, size) = quick_panel_growth(
            work_area(720),
            PhysicalPosition::new(400, 300),
            tauri::PhysicalSize::new(390, 400),
            2_000,
        );
        assert!(position.y >= 12, "the top margin is kept");
        assert!(position.y + size.height as i32 <= 720 - 12, "so is the bottom one");
    }

    #[test]
    fn a_column_is_reserved_in_layout_pixels_whatever_the_display_scales_by() {
        let frame = tauri::PhysicalSize::new(18, 10);
        let standard = settings::QuickPanelDensity::Standard;
        assert_eq!(quick_panel_size(standard, 2, 600, 1.0, frame).width, 780 + 18);
        assert_eq!(quick_panel_size(standard, 2, 600, 1.25, frame).width, 975 + 18);
        assert_eq!(quick_panel_size(standard, 1, 600, 1.0, frame).width, 390 + 18);
        assert_eq!(
            quick_panel_size(standard, 3, 600, 1.0, frame).width,
            780 + 18,
            "two columns is the widest the panel goes"
        );
        assert_eq!(
            quick_panel_size(standard, 2, 600, 1.0, frame).height,
            600,
            "the height is passed through"
        );
    }

    #[test]
    fn the_compact_density_is_one_column_whatever_the_provider_count() {
        let frame = tauri::PhysicalSize::new(18, 10);
        let compact = settings::QuickPanelDensity::Compact;
        for providers in [1, 2, 3] {
            assert_eq!(quick_panel_size(compact, providers, 600, 1.0, frame).width, 180 + 18);
        }
        assert_eq!(quick_panel_size(compact, 2, 600, 1.25, frame).width, 225 + 18);
    }

    #[test]
    fn quick_panel_fits_small_and_common_displays() {
        for (width, height) in [(800, 600), (1280, 720), (1366, 768)] {
            for tray_x in [0.0, width as f64 - 40.0] {
                let (position, size) = placement(width, height, tray_x, height as f64 - 40.0);
                assert!(position.x >= 0 && position.y >= 0);
                assert!(position.x as u32 + size.width <= width);
                assert!(position.y as u32 + size.height <= height);
            }
        }
    }
}
