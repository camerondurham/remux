use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct ExitFailure {
    code: i32,
    message: Option<String>,
}

impl ExitFailure {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: Some(message.into()),
        }
    }

    pub fn quiet(code: i32) -> Self {
        Self {
            code,
            message: None,
        }
    }

    pub fn code(&self) -> i32 {
        self.code
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl fmt::Display for ExitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(message) => f.write_str(message),
            None => write!(f, "exited with status {}", self.code),
        }
    }
}

impl Error for ExitFailure {}
