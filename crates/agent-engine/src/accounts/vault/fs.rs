use std::path::Path;

/// Sao chép thư mục credentials từ nguồn sang đích với phân quyền an toàn.
pub fn copy_credential_dir(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o700));
        }
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_credential_dir(&path, &target)?;
        } else {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let is_credential = file_name.ends_with(".json")
                || file_name.ends_with(".toml")
                || file_name.ends_with(".yaml")
                || file_name.ends_with(".yml")
                || file_name.contains("token")
                || file_name.contains("auth")
                || file_name.contains("cred")
                || file_name.contains("session")
                || file_name.contains("config");
            if is_credential {
                std::fs::copy(&path, &target)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
    Ok(())
}

/// Sao chép thư mục đệ quy (tiện ích I/O).
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    copy_credential_dir(src, dst)
}
