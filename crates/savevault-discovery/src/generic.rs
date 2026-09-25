//! Generic detector: heurística para jogos fora do Ludusavi Manifest.
//! Princípio: preferir falsos negativos a falsos positivos (seção 10 do spec).
//! Implementação completa no v0.4.

use anyhow::Result;
use savevault_core::models::Game;

#[derive(Default)]
pub struct GenericDetector;

impl GenericDetector {
    pub fn new() -> Self {
        Self
    }

    /// TODO v0.4: varrer Documents, AppData/Local, AppData/Roaming buscando
    /// arquivos com extensões típicas de save (.sav, .dat, .save, .sg*) em
    /// subpastas cujo nome coincida com executáveis de jogo conhecidos.
    pub async fn detect(&self) -> Result<Vec<Game>> {
        Ok(vec![])
    }
}
