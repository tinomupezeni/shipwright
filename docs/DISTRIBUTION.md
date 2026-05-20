# Shipwright Distribution

Shipwright is designed for frictionless distribution. While we provide raw binaries, we recommend using package managers for the best experience.

## 🪟 Windows (Recommended)

### Winget
```powershell
winget install shipwright
```

### Scoop
```powershell
scoop bucket add shipwright https://github.com/tinomupezeni/scoop-bucket
scoop install shipwright
```

### Portable (Zero-Dependency)
Download `shipwright-windows-x64.exe` from [GitHub Releases](https://github.com/tinomupezeni/shipwright/releases). 
No installation is required. Move the binary to a folder in your `PATH` to use it globally.

---

## 🐧 Linux & 🍎 macOS

### One-line Install
```bash
curl -sS https://get.shipwright.dev | sh
```

### Manual Binary
1. Download the binary for your platform from GitHub.
2. Make it executable: `chmod +x shipwright`
3. Move to your bin: `mv shipwright /usr/local/bin/`

---

## 🔄 Self-Updating
Shipwright can update itself and its remote daemon automatically.

- **CLI Update**: `shipwright update`
- **Agent Update**: `shipwright update --agent`

The CLI also performs a non-intrusive background check once every 24 hours to notify you of new versions.
