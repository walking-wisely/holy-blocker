#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Language {
    English,
    Ukrainian,
    Unknown,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::English, Language::Ukrainian, Language::Unknown];
}
