use thiserror::Error;

#[derive(Debug, Error)]
pub enum SaveVaultError {
    #[error("Erro de I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("Erro de banco de dados: {0}")]
    Database(String),

    #[error("Erro de configuração: {0}")]
    Config(String),

    #[error("Jogo não encontrado: {0}")]
    GameNotFound(String),

    #[error("Save não encontrado: {0}")]
    SaveNotFound(String),

    #[error("Erro de cloud: {0}")]
    Cloud(String),

    #[error("Erro de hash: {0}")]
    Hash(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SaveVaultError>;
