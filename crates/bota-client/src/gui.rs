//! The debug viewer's own state over the client core: where the eye is.

use std::ops::{Deref, DerefMut};

use bota_client_core::{App, Source};

use crate::camera::{Camera, MAP_SIZE};

/// The client core and the camera over it. Reads and writes of the core go
/// through it as if it were the core.
pub struct Gui {
    /// Everything the client knows.
    pub core: App,
    /// The eye.
    pub camera: Camera,
}

impl Gui {
    /// A viewer over a source, before anything has been heard, looking at
    /// the middle of the map.
    pub fn new(source: Source) -> Gui {
        Gui {
            core: App::new(source),
            camera: Camera::over(MAP_SIZE / 2.0, MAP_SIZE / 2.0),
        }
    }
}

impl Deref for Gui {
    type Target = App;

    fn deref(&self) -> &App {
        &self.core
    }
}

impl DerefMut for Gui {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.core
    }
}
