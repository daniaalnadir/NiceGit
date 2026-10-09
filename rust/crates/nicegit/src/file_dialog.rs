//! Native open and save dialogs. Interface tests cannot drive a native dialog, so under test
//! each dialog takes its answer from `answer_next` instead, or is cancelled when none is set.

use std::path::PathBuf;

#[cfg(test)]
thread_local! {
    static ANSWER: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Makes the next dialog on this thread return `path`, as if the person had chosen it.
#[cfg(test)]
pub fn answer_next(path: impl Into<PathBuf>) {
    ANSWER.with(|answer| *answer.borrow_mut() = Some(path.into()));
}

#[cfg(test)]
fn answer() -> Option<PathBuf> {
    ANSWER.with(|answer| answer.borrow_mut().take())
}

/// Asks for a folder.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = title;
        answer()
    }
    #[cfg(not(test))]
    rfd::FileDialog::new().set_title(title).pick_folder()
}

/// Asks for an existing file whose extension is one of `extensions`.
pub fn pick_file(title: &str, kind: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = (title, kind, extensions);
        answer()
    }
    #[cfg(not(test))]
    rfd::FileDialog::new().set_title(title).add_filter(kind, extensions).pick_file()
}

/// Asks where to save a file, suggesting `name`.
pub fn save_file(title: &str, name: &str) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = (title, name);
        answer()
    }
    #[cfg(not(test))]
    rfd::FileDialog::new().set_title(title).set_file_name(name).save_file()
}
