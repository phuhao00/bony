//! Platform-safe child process creation for the desktop application.

use std::ffi::OsStr;

/// Creates a background child process without opening a console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    hide_console(&mut command);
    command
}

/// Creates a Tokio background child process without opening a console window on Windows.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    hide_console(command.as_std_mut());
    command
}

fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // CREATE_NO_WINDOW. This is required even when the parent binary uses
        // the Windows GUI subsystem, because console child programs otherwise
        // create their own visible terminal window.
        command.creation_flags(0x0800_0000);
    }
}
