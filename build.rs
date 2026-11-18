// Die Oberfläche (Dioxus, `ui/`) wird mit `cargo xtask ui` nach `ui/dist/public` gebaut und hier
// ins Programm eingebettet: Für jede Datei entsteht ein Eintrag in `$OUT_DIR/spa.rs`, Text und
// WebAssembly zusätzlich gzip-komprimiert. Fehlt der Ordner, bleibt die Liste leer und der Server
// zeigt einen Hinweis statt der Oberfläche.
//
// Dasselbe für `static/` (Schrift, Icons): alles, was beim Übersetzen dort liegt, steht unter
// `/static/<pfad>` bereit (`$OUT_DIR/static.rs`).
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=ui/dist/public");
    println!("cargo:rerun-if-changed=static");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Oberfläche
    let root = manifest.join("ui/dist/public");
    let mut files = Vec::new();
    if root.join("index.html").is_file() {
        collect(&root, &root, &mut files);
    }
    files.sort();

    let mut code = String::from("pub static FILES: &[SpaFile] = &[\n");
    for (i, rel) in files.iter().enumerate() {
        let src = root.join(rel);
        println!("cargo:rerun-if-changed={}", src.display());
        let data = std::fs::read(&src).unwrap();
        let gz = if compressible(rel) {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            enc.write_all(&data).unwrap();
            let gz = enc.finish().unwrap();
            let path = out.join(format!("spa-{i}.gz"));
            std::fs::write(&path, gz).unwrap();
            format!("Some(include_bytes!({:?}))", path.display().to_string())
        } else {
            "None".into()
        };
        code += &format!(
            "    SpaFile {{ path: {rel:?}, body: include_bytes!({:?}), gzip: {gz} }},\n",
            src.display().to_string()
        );
    }
    code += "];\n";
    std::fs::write(out.join("spa.rs"), code).unwrap();

    // Statische Dateien (static/)
    let root = manifest.join("static");
    let mut files = Vec::new();
    if root.is_dir() {
        collect(&root, &root, &mut files);
    }
    files.retain(|f| !f.split('/').any(|c| c.starts_with('.')));
    files.sort();
    let mut code = String::from("pub static STATIC_FILES: &[(&str, &[u8])] = &[\n");
    for rel in &files {
        let src = root.join(rel);
        println!("cargo:rerun-if-changed={}", src.display());
        code += &format!(
            "    ({rel:?}, include_bytes!({:?})),\n",
            src.display().to_string()
        );
    }
    code += "];\n";
    std::fs::write(out.join("static.rs"), code).unwrap();
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.push(rel);
        }
    }
}

fn compressible(path: &str) -> bool {
    [".html", ".js", ".css", ".wasm", ".svg", ".json"]
        .iter()
        .any(|e| path.ends_with(e))
}
