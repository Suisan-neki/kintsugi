//! 開発時は固定したUIディレクトリから読み、リリースには同じ資材を埋め込む。
use std::io;
#[cfg(debug_assertions)]
pub(super) fn text(path: &str) -> io::Result<String> {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
}
#[cfg(not(debug_assertions))]
pub(super) fn text(path: &str) -> io::Result<String> {
    let text = match path {
        "ui/app.html" => include_str!("../../ui/app.html"),
        "ui/workbench.html" => include_str!("../../ui/workbench.html"),
        "ui/web-trial.html" => include_str!("../../ui/web-trial.html"),
        "ui/app.js" => include_str!("../../ui/app.js"),
        "ui/workbench.js" => include_str!("../../ui/workbench.js"),
        "ui/web-trial.js" => include_str!("../../ui/web-trial.js"),
        "ui/api.js" => include_str!("../../ui/api.js"),
        "ui/workbench-dom.js" => include_str!("../../ui/workbench-dom.js"),
        "ui/repository-diagram.js" => include_str!("../../ui/repository-diagram.js"),
        "ui/app.css" => include_str!("../../ui/app.css"),
        "ui/dads/tokens.css" => include_str!("../../ui/dads/tokens.css"),
        "ui/dads/components.css" => include_str!("../../ui/dads/components.css"),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "画面の資材がありません",
            ));
        }
    };
    Ok(text.to_owned())
}
pub(super) fn layout() -> io::Result<Vec<u8>> {
    #[cfg(debug_assertions)]
    {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/repository-generated/elk.js.gz"
        ))
    }
    #[cfg(not(debug_assertions))]
    {
        Ok(include_bytes!("../../ui/repository-generated/elk.js.gz").to_vec())
    }
}
