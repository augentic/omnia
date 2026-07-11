use std::io::{self, Write};
use std::process::ExitCode;

/// Buffered command output and process exit status.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandResponse {
    /// Bytes written to standard output.
    pub stdout: Vec<u8>,
    /// Bytes written to standard error.
    pub stderr: Vec<u8>,
    /// Numeric process exit status.
    pub exit: u8,
}

impl CommandResponse {
    /// Create a successful response.
    #[must_use]
    pub fn success(stdout: impl Into<Vec<u8>>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: Vec::new(),
            exit: 0,
        }
    }

    /// Create a failed response.
    #[must_use]
    pub fn failure(stderr: impl Into<Vec<u8>>, exit: u8) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: stderr.into(),
            exit,
        }
    }

    /// Convert the numeric status for a native process boundary.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.exit)
    }

    /// Write both output channels without changing the response exit status.
    ///
    /// # Errors
    ///
    /// Returns the first output sink error.
    pub fn write_to(
        &self, stdout: &mut impl Write, stderr: &mut impl Write,
    ) -> io::Result<ExitCode> {
        stdout.write_all(&self.stdout)?;
        stderr.write_all(&self.stderr)?;
        Ok(self.exit_code())
    }
}
