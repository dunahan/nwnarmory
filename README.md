# NWNArmory

**A Neverwinter Nights Model Rescaling CLI Tool**

NWNArmory is a command-line interface (CLI) tool designed to automatically resize and transform standard Neverwinter Nights (NWN) 3D ASCII `.mdl` models into variants suitable for other playable races (Halfling, Dwarf, Elf, Gnome, Half-orc). 

---

## 🌟 Acknowledgements & Original Developers

**This project is a modern rewrite of the original NWNArmory.** It is of utmost importance to highlight and credit the original creators who made this tool possible. 

*   **Original Author:** Eligio Sacateca (Rumbles Designs), June 2003.
*   **Data Compilation:** Based on original data compiled by **Bearthing**.

The original NWNArmory was a Windows MFC/Visual C++ GUI application. You can still find references to the original logic and instructions in the historical `readme.txt` included in the project history.

## 🚀 The Transition to Rust and CLI

The original C++ codebase has been completely rewritten in **Rust**. This transition brings several major structural changes:

*   **Command-Line Interface (CLI):** The GUI has been completely removed. NWNArmory is now a pure CLI tool, making it significantly easier to integrate into automated build pipelines and batch scripts.
*   **Modern Safety & Performance:** Leveraging Rust's ownership and error-handling models guarantees memory safety and predictable execution without legacy C++ runtime issues.

## ✨ Improvements in the Newest Version (v1.2.1)

Building upon the original v1.2 logic (which introduced absolute translation, minimum/maximum parameters, and texture map scaling), this new Rust-based version (v1.2.1) introduces critical bug fixes and stability improvements:

*   **Safe File Writing (No Partial Writes):** If an error occurs in the middle of processing a `verts` block, the tool no longer leaves a corrupted, partially written file behind. It now writes to a `.tmp` file and only performs an atomic rename upon success.
*   **Collision Prevention:** Fixed a bug where files would silently overwrite each other on name collisions. The tool now tracks target paths per run, issuing a warning and skipping the file if a collision is detected.
*   **Eliminated Legacy IO Errors:** The notorious `CFileException` unpopulated errors from the original codebase are permanently eliminated, structurally guaranteed by Rust's `Result<T, io::Error>` handling.

## 🛠️ Usage

Installation is simple. Once compiled, you can run the binary directly via your terminal:

```bash
nwnarmory <transforms.ini> <source_file_or_folder> <target_folder>
```

*   `<transforms.ini>`: The configuration file containing the scaling and translation matrices (e.g., `standard.ini`).
*   `<source_file_or_folder>`: A single `.mdl` file or a directory containing multiple `.mdl` files.
*   `<target_folder>`: The directory where the transformed models will be saved.

## 📄 License

This project is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0).
