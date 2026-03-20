#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf as StdPathBuf;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::path::BaseDirectory;
use tauri::Manager;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug)]
struct GhostscriptRuntime {
    executable: String,
    gs_lib: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CompressRequest {
    input_path: String,
    output_path: String,
    compression_level: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompressResult {
    input_path: String,
    output_path: String,
    initial_size_bytes: u64,
    final_size_bytes: u64,
    ratio: f64,
}

#[tauri::command]
fn suggest_output_path(input_path: String) -> Result<String, String> {
    let input = PathBuf::from(&input_path);
    if !input.exists() {
        return Err(format!("输入文件不存在: {input_path}"));
    }

    let parent = input
        .parent()
        .ok_or_else(|| "无法解析输入文件所在目录".to_string())?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "无法解析输入文件名".to_string())?;

    Ok(parent
        .join(format!("{stem}_compressed.pdf"))
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
async fn compress_pdf(app: tauri::AppHandle, req: CompressRequest) -> Result<CompressResult, String> {
    tauri::async_runtime::spawn_blocking(move || compress_pdf_inner(&app, req))
        .await
        .map_err(|e| format!("压缩任务执行失败: {e}"))?
}

fn compress_pdf_inner(app: &tauri::AppHandle, req: CompressRequest) -> Result<CompressResult, String> {
    let input = PathBuf::from(&req.input_path);
    let output = PathBuf::from(&req.output_path);

    validate_input(&input)?;
    validate_output(&output)?;

    if input == output {
        return Err("输入与输出路径不能相同，请选择新的输出文件".to_string());
    }

    let quality = match req.compression_level {
        0 => "/default",
        1 => "/prepress",
        2 => "/printer",
        3 => "/ebook",
        4 => "/screen",
        other => return Err(format!("压缩档位无效: {other}，仅支持 0 到 4")),
    };

    let gs_runtime = resolve_ghostscript(&app)?;
    let initial_size_bytes = fs::metadata(&input)
        .map_err(map_io_err("读取输入文件信息失败"))?
        .len();

    let mut command = Command::new(&gs_runtime.executable);
    if let Some(gs_lib) = gs_runtime.gs_lib {
        command.env("GS_LIB", gs_lib);
    }

    let command_output = command
        .args([
            "-sDEVICE=pdfwrite",
            "-dCompatibilityLevel=1.7",
            &format!("-dPDFSETTINGS={quality}"),
            "-dEmbedAllFonts=true",
            "-dSubsetFonts=true",
            "-dNOPAUSE",
            "-dQUIET",
            "-dBATCH",
            &format!("-sOutputFile={}", output.to_string_lossy()),
            &input.to_string_lossy(),
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                "执行 Ghostscript 失败：权限不足，请检查程序和文件权限".to_string()
            } else {
                format!("执行 Ghostscript 失败: {e}")
            }
        })?;

    if !command_output.status.success() {
        let stderr = String::from_utf8_lossy(&command_output.stderr);
        let stdout = String::from_utf8_lossy(&command_output.stdout);
        let details = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            "Ghostscript 返回非零退出码".to_string()
        };
        return Err(format!("PDF 压缩失败：{details}"));
    }

    let final_size_bytes = fs::metadata(&output)
        .map_err(map_io_err("压缩似乎已完成，但读取输出文件失败"))?
        .len();

    let ratio = if initial_size_bytes == 0 {
        0.0
    } else {
        1.0 - (final_size_bytes as f64 / initial_size_bytes as f64)
    };

    Ok(CompressResult {
        input_path: req.input_path,
        output_path: req.output_path,
        initial_size_bytes,
        final_size_bytes,
        ratio,
    })
}

fn validate_input(input: &Path) -> Result<(), String> {
    if !input.exists() {
        return Err(format!("输入文件不存在: {}", input.display()));
    }
    if !input.is_file() {
        return Err(format!("输入路径不是文件: {}", input.display()));
    }
    let ext = input
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "pdf" {
        return Err(format!("输入文件不是 PDF: {}", input.display()));
    }
    Ok(())
}

fn validate_output(output: &Path) -> Result<(), String> {
    let ext = output
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "pdf" {
        return Err(format!("输出文件扩展名必须是 .pdf: {}", output.display()));
    }

    let parent = output
        .parent()
        .ok_or_else(|| "无法解析输出目录".to_string())?;
    if !parent.exists() {
        return Err(format!("输出目录不存在: {}", parent.display()));
    }
    if !parent.is_dir() {
        return Err(format!("输出路径的上级不是目录: {}", parent.display()));
    }

    ensure_writable(parent)?;
    Ok(())
}

fn resolve_ghostscript(app: &tauri::AppHandle) -> Result<GhostscriptRuntime, String> {
    let bundled_candidates = bundled_gs_candidates(app);
    let mut attempted = Vec::new();

    for path in bundled_candidates {
        attempted.push(path.display().to_string());
        if is_usable_ghostscript(&path) {
            return Ok(GhostscriptRuntime {
                executable: path.to_string_lossy().to_string(),
                gs_lib: bundled_gs_lib_env(path.parent()),
            });
        }
    }

    let candidates = ["gs", "gswin64c", "gswin32c"];
    for name in candidates {
        attempted.push(name.to_string());
        match Command::new(name).arg("-version").output() {
            Ok(output) if output.status.success() => {
                return Ok(GhostscriptRuntime {
                    executable: name.to_string(),
                    gs_lib: None,
                })
            }
            Ok(_) => continue,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        }
    }

    let attempted_info = if attempted.is_empty() {
        "(无候选路径)".to_string()
    } else {
        attempted.join("; ")
    };

    Err(format!(
        "未找到可用的 Ghostscript。请确认已内置到 app resources（ghostscript 目录），或确保 gs（gswin64c/gswin32c）在系统 PATH 中。\n已尝试: {attempted_info}"
    ))
}

fn bundled_gs_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut list = Vec::new();

    #[cfg(target_os = "macos")]
    {
        push_unique(&mut list, resolve_resource(app, "ghostscript/macos/bin/gs"));
        push_unique(
            &mut list,
            resolve_resource(app, "resources/ghostscript/macos/bin/gs"),
        );
        push_unique(&mut list, resolve_resource(app, "ghostscript/bin/gs"));
        push_unique(
            &mut list,
            resolve_resource(app, "resources/ghostscript/bin/gs"),
        );

        if let Ok(cwd) = std::env::current_dir() {
            push_unique(
                &mut list,
                cwd.join("src-tauri/resources/ghostscript/macos/bin/gs"),
            );
            push_unique(&mut list, cwd.join("resources/ghostscript/macos/bin/gs"));
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                push_unique(
                    &mut list,
                    parent.join("../Resources/resources/ghostscript/macos/bin/gs"),
                );
                push_unique(
                    &mut list,
                    parent.join("../Resources/ghostscript/macos/bin/gs"),
                );
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        list.push(resolve_resource(
            app,
            "ghostscript/windows/bin/gswin64c.exe",
        ));
        list.push(resolve_resource(
            app,
            "ghostscript/windows/bin/gswin32c.exe",
        ));
        list.push(resolve_resource(
            app,
            "resources/ghostscript/windows/bin/gswin64c.exe",
        ));
        list.push(resolve_resource(
            app,
            "resources/ghostscript/windows/bin/gswin32c.exe",
        ));
        list.push(resolve_resource(app, "ghostscript/bin/gswin64c.exe"));
        list.push(resolve_resource(app, "ghostscript/bin/gswin32c.exe"));
        list.push(resolve_resource(
            app,
            "resources/ghostscript/bin/gswin64c.exe",
        ));
        list.push(resolve_resource(
            app,
            "resources/ghostscript/bin/gswin32c.exe",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        list.push(resolve_resource(app, "ghostscript/linux/bin/gs"));
        list.push(resolve_resource(app, "resources/ghostscript/linux/bin/gs"));
        list.push(resolve_resource(app, "ghostscript/bin/gs"));
        list.push(resolve_resource(app, "resources/ghostscript/bin/gs"));
    }

    list
}

fn push_unique(list: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !list.iter().any(|p| p == &candidate) {
        list.push(candidate);
    }
}

fn bundled_gs_lib_env(bin_dir: Option<&Path>) -> Option<String> {
    let bin_dir = bin_dir?;
    let root = bin_dir.parent()?;

    let share_ghostscript = root.join("share").join("ghostscript");
    if !share_ghostscript.exists() {
        return None;
    }

    let direct_lib = share_ghostscript.join("lib");
    let direct_resource = share_ghostscript.join("Resource");
    let direct_fonts = share_ghostscript.join("fonts");

    let (lib, resource, fonts) =
        if direct_lib.exists() || direct_resource.exists() || direct_fonts.exists() {
            (direct_lib, direct_resource, direct_fonts)
        } else {
            let mut entries = fs::read_dir(&share_ghostscript)
                .ok()?
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());

            let version_dir = entries.pop()?.path();
            (
                version_dir.join("lib"),
                version_dir.join("Resource"),
                version_dir.join("fonts"),
            )
        };

    let mut paths = Vec::<StdPathBuf>::new();
    if lib.exists() {
        paths.push(lib);
    }
    if resource.exists() {
        paths.push(resource);
    }
    if fonts.exists() {
        paths.push(fonts);
    }

    if paths.is_empty() {
        return None;
    }

    std::env::join_paths(paths)
        .ok()
        .map(|joined| joined.to_string_lossy().to_string())
}

fn resolve_resource(app: &tauri::AppHandle, relative: &str) -> PathBuf {
    app.path()
        .resolve(relative, BaseDirectory::Resource)
        .unwrap_or_else(|_| PathBuf::from(relative))
}

fn is_usable_ghostscript(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    #[cfg(unix)]
    {
        if let Ok(metadata) = fs::metadata(path) {
            let mode = metadata.permissions().mode();
            if mode & 0o111 == 0 {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
            }
        }
    }

    match Command::new(path).arg("-version").output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

fn ensure_writable(dir: &Path) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("系统时间异常，无法检测输出目录权限: {e}"))?
        .as_millis();
    let probe = dir.join(format!(".pdf_compress_write_probe_{stamp}.tmp"));

    fs::write(&probe, b"ok").map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!("输出目录无写权限: {}", dir.display())
        } else {
            format!("无法写入输出目录 {}: {e}", dir.display())
        }
    })?;

    fs::remove_file(&probe).map_err(map_io_err("写入权限检测后清理临时文件失败"))?;
    Ok(())
}

fn map_io_err(prefix: &'static str) -> impl Fn(std::io::Error) -> String {
    move |e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!("{prefix}：权限不足")
        } else {
            format!("{prefix}: {e}")
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![compress_pdf, suggest_output_path])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
