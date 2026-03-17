fn main() {
    println!("cargo:warning=================================================");
    println!("cargo:warning=  `cargo install` installs only the binary.");
    println!("cargo:warning=  For full setup (systemd, config, sudoers):");
    println!("cargo:warning=  sudo ./scripts/install.sh");
    println!("cargo:warning=================================================");
}
