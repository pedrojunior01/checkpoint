//! Modelos de dados e parser do Ludusavi Manifest.
//!
//! Mapeia a estrutura de schema oficial do Ludusavi Manifest (schema.yaml)
//! permitindo deserialização resiliente de jogos, caminhos, tags e IDs de loja.

use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::save_detector::SaveDiscoveryError;

/// Representação em memória do Ludusavi Manifest.
///
/// O YAML consiste em um mapeamento onde cada chave é o título de um jogo.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LudusaviManifest {
    #[serde(flatten)]
    pub games: HashMap<String, LudusaviGameEntry>,
}

impl LudusaviManifest {
    /// Carrega e analisa o manifest a partir de uma string YAML.
    pub fn from_yaml_str(yaml: &str) -> Result<Self, SaveDiscoveryError> {
        serde_yaml::from_str(yaml).map_err(|e| SaveDiscoveryError::ParseError(e.to_string()))
    }

    /// Carrega e analisa o manifest a partir de um arquivo no disco.
    pub fn from_path(path: &Path) -> Result<Self, SaveDiscoveryError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_yaml_str(&content)
    }

    /// Retorna o número de jogos cadastrados no manifest.
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// Retorna se o manifest está vazio.
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }
}

/// Metadados de um jogo no Ludusavi Manifest.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LudusaviGameEntry {
    #[serde(default)]
    pub files: HashMap<String, LudusaviFileEntry>,

    #[serde(default)]
    pub registry: HashMap<String, LudusaviRegistryEntry>,

    #[serde(default)]
    pub steam: Option<LudusaviStoreRef>,

    #[serde(default)]
    pub gog: Option<LudusaviStoreRef>,

    #[serde(default, rename = "installDir")]
    pub install_dir: HashMap<String, serde_yaml::Value>,
}

/// Referência para ID de loja (Steam ou GOG).
///
/// No YAML do Ludusavi, IDs podem aparecer como objeto estruturado (`steam: { id: 413150 }`)
/// ou diretamente como valor escalar (`steam: 413150`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LudusaviStoreRef {
    Object { id: StoreIdValue },
    Direct(StoreIdValue),
}

impl LudusaviStoreRef {
    pub fn id_string(&self) -> String {
        match self {
            Self::Object { id } => id.as_str(),
            Self::Direct(id) => id.as_str(),
        }
    }
}

/// Valor de ID de loja (aceita inteiro ou string).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoreIdValue {
    Integer(u64),
    String(String),
}

impl StoreIdValue {
    pub fn as_str(&self) -> String {
        match self {
            Self::Integer(i) => i.to_string(),
            Self::String(s) => s.trim().to_string(),
        }
    }
}

/// Entrada de arquivo ou pasta de save/configuração.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LudusaviFileEntry {
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub when: Vec<LudusaviCondition>,
}

impl LudusaviFileEntry {
    /// Determina se a entrada representa dados de save.
    ///
    /// Se `tags` for vazia ou contiver "save", é considerada save.
    /// Entradas contendo apenas `["config"]` são filtradas.
    pub fn is_save(&self) -> bool {
        if self.tags.is_empty() {
            true
        } else {
            self.tags.iter().any(|t| t.eq_ignore_ascii_case("save"))
        }
    }

    /// Verifica se a regra condicional se aplica ao sistema e loja alvo.
    pub fn is_applicable(&self, target_os: &str, target_store: Option<&str>) -> bool {
        if self.when.is_empty() {
            return true;
        }

        self.when.iter().any(|cond| {
            let os_ok = match &cond.os {
                Some(os) => os.eq_ignore_ascii_case(target_os),
                None => true,
            };

            let store_ok = match (&cond.store, target_store) {
                (Some(expected), Some(actual)) => expected.eq_ignore_ascii_case(actual),
                (Some(_), None) => false,
                (None, _) => true,
            };

            os_ok && store_ok
        })
    }
}

/// Entrada de chave de registro do Windows.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LudusaviRegistryEntry {
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub when: Vec<LudusaviCondition>,
}

/// Condições de restrição (`when`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LudusaviCondition {
    #[serde(default)]
    pub os: Option<String>,

    #[serde(default)]
    pub store: Option<String>,
}
