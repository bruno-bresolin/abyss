use anyhow::{Context, Result, ensure};
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
    process::{Command, Stdio},
};

pub(super) fn install_missing() -> Result<bool> {
    if !cfg!(target_os = "linux") {
        return Ok(false);
    }
    let release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let id = release
        .lines()
        .find_map(|line| line.strip_prefix("ID="))
        .unwrap_or_default()
        .trim_matches(['"', '\'']);
    let (manager, packages) = match id {
        "debian" | "ubuntu" => ("/usr/bin/apt-get", &["libvulkan1", "mesa-vulkan-drivers"]),
        "fedora" => ("/usr/bin/dnf", &["vulkan-loader", "mesa-vulkan-drivers"]),
        _ => return Ok(false),
    };
    let mut missing = Vec::new();
    for package in packages {
        let mut query = if id == "fedora" {
            let mut rpm = Command::new("/usr/bin/rpm");
            rpm.args(["-q", package]);
            rpm
        } else {
            let mut dpkg = Command::new("/usr/bin/dpkg-query");
            dpkg.args(["--show", "--showformat=${db:Status-Status}", package]);
            dpkg
        };
        let output = query.output().context("consultando pacotes Vulkan")?;
        let installed =
            output.status.success() && (id == "fedora" || output.stdout == b"installed");
        if !installed {
            missing.push(*package);
        }
    }
    if missing.is_empty() || !Path::new(manager).is_file() {
        return Ok(false);
    }
    let mut stderr = io::stderr();
    let stdin = io::stdin();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        eprintln!(
            "Pacotes Vulkan ausentes: {}. Abra o Abyss em um terminal interativo para autorizar a instalação.",
            missing.join(", ")
        );
        return Ok(false);
    }
    write!(
        stderr,
        "Autoriza executar `sudo {manager} install {}`? [s/N] ",
        missing.join(" ")
    )?;
    stderr.flush()?;
    let mut answer = String::new();
    stdin.read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "s" | "sim") {
        return Ok(false);
    }
    let status = Command::new("/usr/bin/sudo")
        .args([manager, "install"])
        .args(missing)
        .stdin(Stdio::inherit())
        .status()
        .context("executando sudo para instalar os pacotes Vulkan")?;
    ensure!(status.success(), "instalação Vulkan terminou com {status}");
    Ok(true)
}
