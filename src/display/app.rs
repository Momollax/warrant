use crate::models::warrant::WarrantSnapshot;

/// Résultat d'un ticker : données ou message d'erreur.
pub enum WarrantRow {
    Data(WarrantSnapshot),
    Error(String),
}

/// État global de l'application TUI.
pub struct App {
    pub rows:          Vec<WarrantRow>,
    pub cycle:         u32,
    pub status:        String,
    pub should_quit:   bool,
    pub force_refresh: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            rows:          Vec::new(),
            cycle:         0,
            status:        String::from("Initialisation…"),
            should_quit:   false,
            force_refresh: false,
        }
    }
}
