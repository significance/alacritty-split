use serde::Serialize;

use alacritty_config_derive::ConfigDeserialize;

use crate::display::color::Rgb;

/// Styling for native split panes. Dimensions are logical pixels.
#[derive(ConfigDeserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Panes {
    pub padding: u16,
    pub divider_thickness: u16,
    pub divider_color: Option<Rgb>,
    pub focused_edge_thickness: u16,
    pub focused_edge_color: Option<Rgb>,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            padding: 1,
            divider_thickness: 1,
            divider_color: None,
            focused_edge_thickness: 1,
            focused_edge_color: None,
        }
    }
}

impl Panes {
    pub fn physical_thickness(logical: u16, scale: f32) -> u32 {
        if logical == 0 { 0 } else { (f32::from(logical) * scale).round().max(1.) as u32 }
    }

    pub fn divider_pixels(&self, scale: f32) -> u32 {
        Self::physical_thickness(self.divider_thickness, scale)
    }

    pub fn edge_pixels(&self, scale: f32) -> u32 {
        Self::physical_thickness(self.focused_edge_thickness, scale)
    }

    pub fn padding_pixels(&self, scale: f32) -> f32 {
        Self::physical_thickness(self.padding, scale) as f32
    }

    pub fn focused_edge_color(&self) -> Rgb {
        self.focused_edge_color.unwrap_or(Rgb::new(0, 0, 0))
    }

    pub fn divider_color(&self, foreground: Rgb, background: Rgb) -> Rgb {
        self.divider_color.unwrap_or_else(|| {
            Rgb::new(
                ((u16::from(foreground.r) + 3 * u16::from(background.r)) / 4) as u8,
                ((u16::from(foreground.g) + 3 * u16::from(background.g)) / 4) as u8,
                ((u16::from(foreground.b) + 3 * u16::from(background.b)) / 4) as u8,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UiConfig;

    #[test]
    fn default_pane_style_preserves_borders_and_adds_one_pixel_padding() {
        let config: UiConfig = toml::from_str("").unwrap();
        assert_eq!(config.panes, Panes::default());
        assert_eq!(config.panes.padding_pixels(1.), 1.);
        assert_eq!(config.panes.padding_pixels(2.), 2.);
        assert_eq!(config.panes.divider_pixels(1.), 1);
        assert_eq!(config.panes.edge_pixels(2.), 2);
        assert_eq!(
            config.panes.divider_color(Rgb::new(200, 100, 60), Rgb::new(0, 0, 0)),
            Rgb::new(50, 25, 15)
        );
    }

    #[test]
    fn pane_style_config_parses_and_scales_with_zero_disabling_edges() {
        let config: UiConfig = toml::from_str(
            r##"
[panes]
padding = 3
divider_thickness = 2
divider_color = "#123456"
focused_edge_thickness = 0
focused_edge_color = "#abcdef"
"##,
        )
        .unwrap();
        assert_eq!(config.panes.padding_pixels(2.), 6.);
        assert_eq!(config.panes.divider_pixels(1.5), 3);
        assert_eq!(config.panes.edge_pixels(2.), 0);
        assert_eq!(config.panes.divider_color, Some(Rgb::new(0x12, 0x34, 0x56)));
        assert_eq!(config.panes.focused_edge_color, Some(Rgb::new(0xab, 0xcd, 0xef)));
    }
}
