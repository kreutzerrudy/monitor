use crate::source::StreamSource;

pub type LayerId = u64;

pub struct Layer {
    pub id: LayerId,
    pub z_index: i32,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub source: Box<dyn StreamSource>,
}
