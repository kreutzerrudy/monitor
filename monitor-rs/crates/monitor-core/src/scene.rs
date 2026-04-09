use crate::layer::{Layer, LayerId};
use crate::source::StreamSource;

/// Ordered collection of layers. Kept sorted by ascending `z_index` so the
/// compositor can iterate bottom-to-top (canvas first, overlays last).
pub struct Scene {
    layers: Vec<Layer>,
    next_id: u64,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            next_id: 1,
        }
    }

    /// Add a layer and return its assigned ID. The scene re-sorts after insertion.
    pub fn add(
        &mut self,
        source: Box<dyn StreamSource>,
        z_index: i32,
        x: i32,
        y: i32,
    ) -> LayerId {
        let id = self.next_id;
        self.next_id += 1;
        self.layers.push(Layer {
            id,
            z_index,
            x,
            y,
            visible: true,
            source,
        });
        self.layers.sort_by_key(|l| l.z_index);
        id
    }

    /// Remove a layer by ID. Returns the removed layer (with its source) or `None`.
    pub fn remove(&mut self, id: LayerId) -> Option<Layer> {
        let pos = self.layers.iter().position(|l| l.id == id)?;
        Some(self.layers.remove(pos))
    }

    /// Change a layer's z-index and re-sort.
    pub fn reorder(&mut self, id: LayerId, new_z: i32) -> bool {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.z_index = new_z;
            self.layers.sort_by_key(|l| l.z_index);
            true
        } else {
            false
        }
    }

    /// Update a layer's canvas position.
    pub fn set_position(&mut self, id: LayerId, x: i32, y: i32) -> bool {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.x = x;
            layer.y = y;
            true
        } else {
            false
        }
    }

    /// Update a layer's visibility.
    pub fn set_visible(&mut self, id: LayerId, visible: bool) -> bool {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.visible = visible;
            true
        } else {
            false
        }
    }

    /// Iterate layers in ascending z-order (bottom to top).
    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter()
    }

    /// Mutable access to a layer by ID.
    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Remove all layers, stopping each source. Returns the removed layers.
    pub fn clear(&mut self) -> Vec<Layer> {
        std::mem::take(&mut self.layers)
    }
}
