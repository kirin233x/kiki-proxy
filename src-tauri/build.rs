fn main() {
    // 把 Git 提交号和构建时间编译进 App，设置页底部会显示，方便确认运行的是哪个版本
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|o| !o.stdout.is_empty());
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=KIKI_GIT_HASH={hash}{}", if dirty { "+改动" } else { "" });
    println!("cargo:rustc-env=KIKI_BUILD_TIME={secs}");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    tauri_build::build()
}
