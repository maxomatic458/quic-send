use std::path::Path;

#[tauri::command(async)]
pub async fn get_file_size_and_is_dir(path: &Path) -> Result<(u64, bool), String> {
    println!("getting file size and is dir for {:?}", path);
    let metadata = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
    Ok((metadata.len(), metadata.is_dir()))
}
