use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalRect, Rect};

use macrdp_core::Metrics;

/// Set up the system tray icon event handling.
///
/// The tray icon itself is created from `tauri.conf.json` (id: "main-tray").
/// This function retrieves it and registers the click handler to toggle
/// the popover window.
pub fn setup_tray(handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let tray = handle
        .tray_by_id("main-tray")
        .ok_or("tray icon 'main-tray' not found")?;

    tray.set_tooltip(Some("macrdp"))?;

    let app_handle = handle.clone();
    tray.on_tray_icon_event(move |_tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            rect,
            ..
        } = event
        {
            toggle_popover(&app_handle, rect);
        }
    });

    Ok(())
}

/// Toggle the popover window visibility.
///
/// If the popover is currently visible, hide it.
/// If hidden, position it below the clicked tray icon, then show and focus it.
fn toggle_popover(handle: &AppHandle, tray_rect: Rect) {
    if let Some(window) = handle.get_webview_window("popover") {
        match window.is_visible() {
            Ok(true) => {
                let _ = window.hide();
            }
            _ => {
                if let Err(error) = position_popover_near_tray(handle, &window, tray_rect) {
                    tracing::warn!(%error, "Failed to position tray popover");
                }
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }
}

/// Anchor to the clicked status item using its screen's logical coordinates.
fn position_popover_near_tray(
    handle: &AppHandle,
    window: &tauri::WebviewWindow,
    tray_rect: Rect,
) -> anyhow::Result<()> {
    // On macOS, cursor_position uses the primary screen's scale, while
    // monitor_from_point takes global logical coordinates. The tray rectangle
    // and monitor work area use the clicked screen's scale instead.
    let primary = handle
        .primary_monitor()?
        .ok_or_else(|| anyhow::anyhow!("No primary monitor"))?;
    let cursor = handle
        .cursor_position()?
        .to_logical::<f64>(primary.scale_factor());
    let monitor = handle
        .monitor_from_point(cursor.x, cursor.y)?
        .ok_or_else(|| anyhow::anyhow!("No monitor at tray click"))?;
    let window_size = window
        .outer_size()?
        .to_logical::<f64>(window.scale_factor()?);
    let position = popover_position(
        tray_rect,
        window_size,
        *monitor.work_area(),
        monitor.scale_factor(),
    );
    // Logical coordinates keep the size/position consistent when the hidden
    // popover was last shown on a screen with a different scale factor.
    window.set_position(position)?;
    Ok(())
}

/// Normalize native geometry once, then center below the item and keep it visible.
fn popover_position(
    tray_rect: Rect,
    window_size: LogicalSize<f64>,
    work_area: PhysicalRect<i32, u32>,
    scale: f64,
) -> LogicalPosition<f64> {
    const GAP: f64 = 4.0;
    let tray_position = tray_rect.position.to_logical::<f64>(scale);
    let tray_size = tray_rect.size.to_logical::<f64>(scale);
    let screen_position = work_area.position.to_logical::<f64>(scale);
    let screen_size = work_area.size.to_logical::<f64>(scale);
    let left = screen_position.x + GAP;
    let top = screen_position.y + GAP;
    let right = (screen_position.x + screen_size.width - window_size.width - GAP).max(left);
    let bottom = (screen_position.y + screen_size.height - window_size.height - GAP).max(top);
    LogicalPosition::new(
        (tray_position.x + (tray_size.width - window_size.width) / 2.0).clamp(left, right),
        (tray_position.y + tray_size.height + GAP).clamp(top, bottom),
    )
}

/// Update the tray icon title to reflect the current server metrics.
///
/// On macOS the tray title is displayed as text next to the icon:
/// - When there is a connection (bitrate > 0) — show bitrate and latency
/// - Otherwise — clear the title
pub fn update_tray_status(
    handle: &AppHandle,
    state: &str,
    metrics: &Metrics,
) {
    let Some(tray) = handle.tray_by_id("main-tray") else {
        return;
    };

    match state {
        "connected" => {
            let mbps = metrics.bitrate_kbps as f64 / 1000.0;
            let ms = metrics.rtt_ms;
            let title = format!("{:.1} Mbps | {:.0} ms", mbps, ms);
            let _ = tray.set_title(Some(&title));
        }
        _ => {
            let _ = tray.set_title(None::<&str>);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popover_follows_the_icon_and_actual_width_at_each_scale() {
        for scale in [1.0, 2.0] {
            let work_area = PhysicalRect {
                position: LogicalPosition::new(0.0, 24.0).to_physical(scale),
                size: LogicalSize::new(1920.0, 1056.0).to_physical(scale),
            };
            for (icon_x, icon_width, window_width, expected_x) in [
                (800.0, 24.0, 300.0, 662.0),
                (1200.0, 24.0, 300.0, 1062.0),
                (800.0, 140.0, 340.0, 700.0),
            ] {
                let tray = Rect {
                    position: LogicalPosition::new(icon_x, 0.0)
                        .to_physical::<i32>(scale)
                        .into(),
                    size: LogicalSize::new(icon_width, 24.0)
                        .to_physical::<u32>(scale)
                        .into(),
                };
                assert_eq!(
                    popover_position(
                        tray,
                        LogicalSize::new(window_width, 280.0),
                        work_area,
                        scale
                    ),
                    LogicalPosition::new(expected_x, 28.0)
                );
            }
        }
    }

    #[test]
    fn popover_stays_on_the_clicked_screen_at_its_edges() {
        for (screen_x, screen_y, icon_x, expected_x) in [
            (0.0, 0.0, 2.0, 4.0),
            (0.0, 0.0, 1880.0, 1616.0),
            (-1920.0, -1080.0, -1918.0, -1916.0),
            (1920.0, 400.0, 3800.0, 3536.0),
        ] {
            let work_area = PhysicalRect {
                position: LogicalPosition::new(screen_x, screen_y + 24.0).to_physical(1.0),
                size: LogicalSize::new(1920.0, 1056.0).to_physical(1.0),
            };
            let tray = Rect {
                position: LogicalPosition::new(icon_x, screen_y).into(),
                size: LogicalSize::new(24.0, 24.0).into(),
            };
            assert_eq!(
                popover_position(tray, LogicalSize::new(300.0, 280.0), work_area, 1.0),
                LogicalPosition::new(expected_x, screen_y + 28.0)
            );
        }
    }
}
