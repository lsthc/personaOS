#![no_std]

#[derive(Clone, Copy)]
pub struct TextStyle {
    pub id: u64,
    pub line_height: u32,
    pub color: u32,
}

pub const BODY: TextStyle = TextStyle {
    id: 1,
    line_height: 16,
    color: 0xE6E6FA,
};

pub const CAPTION: TextStyle = TextStyle {
    id: 2,
    line_height: 16,
    color: 0x9FB4FF,
};

pub fn style_name(id: u64) -> &'static [u8] {
    match id {
        1 => b"Skipstone Body",
        2 => b"Skipstone Caption",
        _ => b"Skipstone Unknown",
    }
}
