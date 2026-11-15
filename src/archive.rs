use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

pub fn is_supported(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "zip" | "7z" | "rar"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    match extension(archive).as_str() {
        "zip" => {
            let file = fs::File::open(archive)?;
            let mut zip = zip::ZipArchive::new(file).context("Couldn't read the zip file")?;
            zip.extract(dest).context("Couldn't extract the zip file")?;
        }
        "7z" => {
            sevenz_rust2::decompress_file(archive, dest).context("Couldn't extract the 7z file")?;
        }
        "rar" => {
            let mut rar = unrar::Archive::new(archive)
                .open_for_processing()
                .context("Couldn't read the RAR file")?;
            while let Some(header) = rar.read_header().context("Couldn't read the RAR file")? {
                rar = if header.entry().is_file() {
                    header.extract_with_base(dest).context("Couldn't extract the RAR file")?
                } else {
                    header.skip().context("Couldn't extract the RAR file")?
                };
            }
        }
        other => bail!("\"{other}\" files aren't supported. Use a .zip or .7z archive, or a folder."),
    }
    Ok(())
}
