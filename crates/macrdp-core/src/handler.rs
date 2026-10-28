use ironrdp_server::{KeyboardEvent, MouseEvent, RdpServerInputHandler};
use macrdp_input::{KeyboardInjector, MouseButton, MouseInjector};
use std::sync::{Arc, Mutex};

/// Maps RDP desktop coordinates to macOS logical coordinates.
///
/// Uses the captured content bounds inside the RDP desktop, excluding any bars.
/// Formula: `mac = (rdp - content_origin) × cg_logical ÷ content_size`.
#[derive(Debug, Clone)]
pub struct MouseCoordMapper(Arc<Mutex<CoordState>>);

#[derive(Debug)]
struct CoordState {
    logical_w: f64,
    logical_h: f64,
    rdp_w: f64,
    rdp_h: f64,
    content_rect: Option<(f64, f64, f64, f64)>,
}

impl MouseCoordMapper {
    pub fn new(logical_w: u16, logical_h: u16, rdp_w: u16, rdp_h: u16) -> Self {
        tracing::info!(
            logical_w, logical_h, rdp_w, rdp_h,
            "MouseCoordMapper initialized"
        );
        Self(Arc::new(Mutex::new(CoordState {
            logical_w: logical_w as f64,
            logical_h: logical_h as f64,
            rdp_w: rdp_w.max(1) as f64,
            rdp_h: rdp_h.max(1) as f64,
            content_rect: None,
        })))
    }

    pub fn map(&self, rdp_x: u16, rdp_y: u16) -> (u16, u16) {
        let s = self.0.lock().unwrap();
        let (x, y, w, h) = s.content_rect.unwrap_or((0.0, 0.0, s.rdp_w, s.rdp_h));
        let mac_x = ((rdp_x as f64 - x) * s.logical_w / w)
            .clamp(0.0, (s.logical_w - 1.0).max(0.0)) as u16;
        let mac_y = ((rdp_y as f64 - y) * s.logical_h / h)
            .clamp(0.0, (s.logical_h - 1.0).max(0.0)) as u16;
        (mac_x, mac_y)
    }

    /// RDP pixels per macOS logical point (horizontal).
    pub fn scale(&self) -> f64 {
        let s = self.0.lock().unwrap();
        s.rdp_w / s.logical_w.max(1.0)
    }

    pub fn logical_size(&self) -> (u16, u16) {
        let s = self.0.lock().unwrap();
        (s.logical_w as u16, s.logical_h as u16)
    }

    /// Both coordinate spaces come from the same frame and are checked at capture.
    pub fn update_capture_geometry(
        &self,
        content_rect: Option<(f64, f64, f64, f64)>,
        logical_size: Option<(f64, f64)>,
    ) {
        let mut s = self.0.lock().unwrap();
        let (logical_w, logical_h) = logical_size.unwrap_or((s.logical_w, s.logical_h));
        if s.content_rect != content_rect || s.logical_w != logical_w || s.logical_h != logical_h {
            s.logical_w = logical_w;
            s.logical_h = logical_h;
            s.content_rect = content_rect;
            tracing::debug!(
                "Mouse mapping updated: logical={}x{}, rdp={}x{}, content_pixels={content_rect:?}",
                s.logical_w, s.logical_h, s.rdp_w, s.rdp_h,
            );
        }
    }

    pub fn update_rdp_size(&self, rdp_w: u16, rdp_h: u16) {
        let mut s = self.0.lock().unwrap();
        let old_w = s.rdp_w;
        let old_h = s.rdp_h;
        s.rdp_w = rdp_w.max(1) as f64;
        s.rdp_h = rdp_h.max(1) as f64;
        s.content_rect = None;
        tracing::info!(
            new_rdp_w = rdp_w, new_rdp_h = rdp_h,
            old_rdp_w = old_w, old_rdp_h = old_h,
            logical_w = s.logical_w, logical_h = s.logical_h,
            "MouseCoordMapper: RDP desktop size updated"
        );
    }
}

/// Bridges RDP input events to macOS CGEvent injection
pub struct MacInputHandler {
    keyboard: Option<KeyboardInjector>,
    mouse: Option<MouseInjector>,
    last_mouse_x: u16,
    last_mouse_y: u16,
    coord_mapper: MouseCoordMapper,
}

impl MacInputHandler {
    pub fn new(coord_mapper: MouseCoordMapper) -> Self {
        let keyboard = KeyboardInjector::new()
            .map_err(|e| tracing::error!("Failed to create keyboard injector: {e}"))
            .ok();
        let mouse = MouseInjector::new()
            .map_err(|e| tracing::error!("Failed to create mouse injector: {e}"))
            .ok();

        if keyboard.is_none() || mouse.is_none() {
            tracing::warn!(
                "Input injection may fail — ensure Accessibility permission is granted"
            );
        }

        Self {
            keyboard,
            mouse,
            last_mouse_x: 0,
            last_mouse_y: 0,
            coord_mapper,
        }
    }
}

impl RdpServerInputHandler for MacInputHandler {
    fn keyboard(&mut self, event: KeyboardEvent) {
        let Some(kb) = &mut self.keyboard else { return };

        let result = match event {
            KeyboardEvent::Pressed { code, extended } => kb.inject_key(code, extended, true),
            KeyboardEvent::Released { code, extended } => kb.inject_key(code, extended, false),
            KeyboardEvent::UnicodePressed(ch) => kb.inject_unicode(ch, true),
            KeyboardEvent::UnicodeReleased(ch) => kb.inject_unicode(ch, false),
            KeyboardEvent::Synchronize(flags) => {
                kb.synchronize(flags.bits() & 0x04 != 0); // RDP CAPS_LOCK toggle
                Ok(())
            }
        };

        if let Err(e) = result {
            tracing::warn!("Keyboard injection failed: {e}");
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        let Some(m) = &mut self.mouse else { return };

        let result = match event {
            MouseEvent::Move { x, y } => {
                let (mx, my) = self.coord_mapper.map(x, y);
                self.last_mouse_x = mx;
                self.last_mouse_y = my;
                m.move_to(mx, my)
            }
            MouseEvent::LeftPressed => {
                m.button_event(MouseButton::Left, true, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::LeftReleased => {
                m.button_event(MouseButton::Left, false, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::RightPressed => {
                m.button_event(MouseButton::Right, true, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::RightReleased => {
                m.button_event(MouseButton::Right, false, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::MiddlePressed => {
                m.button_event(MouseButton::Middle, true, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::MiddleReleased => {
                m.button_event(MouseButton::Middle, false, self.last_mouse_x, self.last_mouse_y)
            }
            MouseEvent::VerticalScroll { value } => m.scroll(value, 0),
            MouseEvent::Scroll { x, y } => m.scroll(
                y.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
                x.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
            ),
            _ => {
                tracing::trace!(?event, "Unhandled mouse event");
                Ok(())
            }
        };

        if let Err(e) = result {
            tracing::warn!("Mouse injection failed: {e}");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_mapping_excludes_bars_and_clamps_to_the_visible_desktop() {
        let mapper = MouseCoordMapper::new(1920, 1080, 1920, 1440);
        // A 16:9 desktop inside a 4:3 RDP frame has 180-pixel top/bottom bars.
        mapper.update_capture_geometry(Some((0.0, 180.0, 1920.0, 1080.0)), None);
        assert_eq!(mapper.map(320, 180), (320, 0));
        assert_eq!(mapper.map(960, 720), (960, 540));
        assert_eq!(mapper.map(1919, 1259), (1919, 1079));
        assert_eq!(mapper.map(320, 60), (320, 0));
        assert_eq!(mapper.map(320, 1380), (320, 1079));

        // Use the frame's actual origin and size, including non-centered placement.
        mapper.update_rdp_size(1600, 900);
        mapper.update_capture_geometry(Some((100.0, 80.0, 1280.0, 720.0)), None);
        assert_eq!(mapper.map(100, 80), (0, 0));
        assert_eq!(mapper.map(740, 440), (960, 540));
        assert_eq!(mapper.map(0, 440), (0, 540));
        assert_eq!(mapper.map(1599, 440), (1919, 540));

        // The capture boundary converts Retina surface points to output pixels.
        mapper.update_rdp_size(3840, 2160);
        mapper.update_capture_geometry(Some((0.0, 0.0, 3840.0, 2160.0)), None);
        assert_eq!(mapper.map(0, 0), (0, 0));
        assert_eq!(mapper.map(960, 540), (480, 270));
        assert_eq!(mapper.map(1920, 1080), (960, 540));
        assert_eq!(mapper.map(3839, 2159), (1919, 1079));

        mapper.update_rdp_size(3840, 2880);
        mapper.update_capture_geometry(Some((0.0, 360.0, 3840.0, 2160.0)), None);
        assert_eq!(mapper.map(1920, 360), (960, 0));
        assert_eq!(mapper.map(1920, 1440), (960, 540));
        assert_eq!(mapper.map(1920, 2519), (960, 1079));

        // A live display mode change must update the source height as well as bars.
        // These are the observed SCK geometries at a fixed 3840x2160 output size.
        mapper.update_rdp_size(3840, 2160);
        mapper.update_capture_geometry(
            Some((300.0, 0.0, 3240.0, 2160.0)), Some((1920.0, 1280.0)),
        );
        assert_eq!(mapper.map(1920, 1080), (960, 640));
        assert_eq!(mapper.map(1920, 2159), (960, 1279));
        mapper.update_capture_geometry(
            Some((0.0, 0.0, 3840.0, 2160.0)), Some((1920.0, 1080.0)),
        );
        for (rdp_y, mac_y) in [(0, 0), (540, 270), (1080, 540), (1620, 810), (1822, 911), (2159, 1079)] {
            assert_eq!(mapper.map(1920, rdp_y), (960, mac_y));
        }
    }

    #[test]
    fn mouse_mapping_restores_full_frame_for_fallback_and_resize() {
        let mapper = MouseCoordMapper::new(1920, 1080, 3840, 2160);
        assert_eq!(mapper.map(1920, 1080), (960, 540));
        assert_eq!(mapper.map(3839, 2159), (1919, 1079));
        mapper.update_capture_geometry(Some((0.0, 180.0, 1920.0, 1080.0)), None);
        mapper.update_capture_geometry(None, Some((1920.0, 1080.0))); // CoreGraphics
        assert_eq!(mapper.map(1920, 1080), (960, 540));

        mapper.update_capture_geometry(Some((0.0, 180.0, 1920.0, 1080.0)), None);
        mapper.update_rdp_size(2560, 1440);
        assert_eq!(mapper.map(1280, 720), (960, 540));
    }
}
