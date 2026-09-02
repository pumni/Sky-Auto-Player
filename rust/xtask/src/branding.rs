use crate::Result;
use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use roxmltree::Document;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use walkdir::WalkDir;

const EXPECTED_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const ICO_ORDER: [u32; 10] = [32, 16, 20, 24, 40, 48, 64, 96, 128, 256];
const APPROVED_BRAND_COMMIT: &str = "1d850a6cd4f7ace2702116159ef788b7e847acea";
const APPROVED_BRAND_SOURCES: [(&str, &str, &str); 3] = [
    (
        "sky-auto-player-app-icon.svg",
        "94f924b540c1927e84eb5aa0938dd29630c5fd55",
        "006f235417081f3394ec458e19eb99780feecd58d1a41742df77552bc018dc6d",
    ),
    (
        "sky-auto-player-app-icon-small.svg",
        "8e0341ae2e87382471b7875a6d2100b5ab2b323f",
        "59c422e5897e1b58637ca6e652b275d692b26f44a44eb7ad8b648c60dcc4606e",
    ),
    (
        "sky-auto-player-app-icon-16.svg",
        "e5112a89662f8eaec5e430628f721526cc01b443",
        "cd9dc8b59ee8f448f9587e22ad855f51af6236204c85197a144ce164b714d353",
    ),
];
const SVG_NS: &str = "http://www.w3.org/2000/svg";

const WINDOWS_RASTER_SOURCES: [(&str, &str); 10] = [
    ("16", "sky-auto-player-app-icon-16.svg"),
    ("20", "sky-auto-player-app-icon-small.svg"),
    ("24", "sky-auto-player-app-icon-small.svg"),
    ("32", "sky-auto-player-app-icon.svg"),
    ("40", "sky-auto-player-app-icon.svg"),
    ("48", "sky-auto-player-app-icon.svg"),
    ("64", "sky-auto-player-app-icon.svg"),
    ("96", "sky-auto-player-app-icon.svg"),
    ("128", "sky-auto-player-app-icon.svg"),
    ("256", "sky-auto-player-app-icon.svg"),
];
const TOOLBAR_RASTER_SOURCES: [(&str, &str); 4] = [
    ("32", "sky-auto-player-app-icon.svg"),
    ("40", "sky-auto-player-app-icon.svg"),
    ("48", "sky-auto-player-app-icon.svg"),
    ("64", "sky-auto-player-app-icon.svg"),
];

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalized_brand_source_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            if bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            normalized.push(b'\n');
        } else {
            normalized.push(bytes[index]);
        }
        index += 1;
    }
    normalized
}

pub fn png_dimensions(data: &[u8]) -> Result<(u32, u32)> {
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" || &data[12..16] != b"IHDR" {
        return Err("expected a PNG file with an IHDR chunk".into());
    }
    Ok((
        u32::from_be_bytes(data[16..20].try_into()?),
        u32::from_be_bytes(data[20..24].try_into()?),
    ))
}

pub fn find_layers(directory: &Path, sizes: &[u32]) -> Result<BTreeMap<u32, Vec<u8>>> {
    if !directory.is_dir() {
        return Err(format!("PNG directory does not exist: {}", directory.display()).into());
    }
    let mut layers = BTreeMap::new();
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("png")
        {
            continue;
        }
        let data = fs::read(entry.path())?;
        let (width, height) = png_dimensions(&data)?;
        if width != height || !sizes.contains(&width) {
            continue;
        }
        if layers.insert(width, data).is_some() {
            return Err(
                format!("duplicate {width}x{height} PNG in {}", directory.display()).into(),
            );
        }
    }
    Ok(layers)
}

pub fn build_ico(layers: &BTreeMap<u32, Vec<u8>>) -> Result<Vec<u8>> {
    let missing = EXPECTED_SIZES
        .iter()
        .copied()
        .filter(|size| !layers.contains_key(size))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing ICO layers: {}",
            missing
                .iter()
                .map(|size| format!("{size}x{size}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    let mut directory = IconDir::new(ResourceType::Icon);
    for size in ICO_ORDER {
        let png = layers.get(&size).expect("checked above");
        let image = IconImage::read_png(Cursor::new(png))?;
        if (image.width(), image.height()) != (size, size) {
            return Err(format!(
                "PNG layer is {}x{}, expected {size}x{size}",
                image.width(),
                image.height()
            )
            .into());
        }
        let entry = if size == 256 {
            IconDirEntry::encode_as_png(&image)?
        } else {
            IconDirEntry::encode_as_bmp(&image)?
        };
        directory.add_entry(entry);
    }
    let mut output = Vec::new();
    directory.write(&mut output)?;
    Ok(output)
}

pub fn build_ico_from_dir(directory: &Path) -> Result<Vec<u8>> {
    let layers = find_layers(directory, &EXPECTED_SIZES)?;
    build_ico(&layers)
}

pub fn write_ico(directory: &Path, output: &Path) -> Result<()> {
    let bytes = build_ico_from_dir(directory)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes)?;
    Ok(())
}

fn parse_svg(path: &Path) -> Result<(String, Document<'static>)> {
    let text = fs::read_to_string(path)?;
    let owned: &'static str = Box::leak(text.into_boxed_str());
    let document = roxmltree::Document::parse(owned)
        .map_err(|error| format!("{}: invalid SVG: {error}", path.display()))?;
    Ok((owned.to_owned(), document))
}

fn validate_brand_lock(root: &Path) -> Result<()> {
    let lock_path = root.join("branding/approved-brand.lock.json");
    let lock: Value = serde_json::from_slice(&fs::read(&lock_path)?)?;
    if lock.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("approved brand lock schema changed".into());
    }
    if lock.get("approved_commit").and_then(Value::as_str) != Some(APPROVED_BRAND_COMMIT) {
        return Err(format!("{}: approved commit changed", lock_path.display()).into());
    }
    let sources = lock
        .get("sources")
        .and_then(Value::as_object)
        .ok_or("approved brand lock sources must be an object")?;
    if sources.len() != APPROVED_BRAND_SOURCES.len() {
        return Err("approved brand lock source set changed".into());
    }
    for (name, git_blob_sha1, sha256) in APPROVED_BRAND_SOURCES {
        let entry = sources
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("approved brand lock entry missing: {name}"))?;
        if entry.get("git_blob_sha1").and_then(Value::as_str) != Some(git_blob_sha1)
            || entry.get("sha256").and_then(Value::as_str) != Some(sha256)
        {
            return Err(format!("approved brand lock metadata changed: {name}").into());
        }
        let bytes = fs::read(root.join("branding").join(name))?;
        let actual = sha256_hex(&normalized_brand_source_bytes(&bytes));
        if actual != sha256 {
            return Err(format!(
                "{} differs from the owner-approved brand source",
                root.join("branding").join(name).display()
            )
            .into());
        }
    }
    Ok(())
}

fn validate_raster_source_section(
    manifest: &Value,
    section: &str,
    expected: &[(&str, &str)],
) -> Result<()> {
    let object = manifest
        .get(section)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("raster source manifest section missing: {section}"))?;
    if object.len() != expected.len() {
        return Err(format!("raster source manifest section changed: {section}").into());
    }
    for (size, source) in expected {
        if object.get(*size).and_then(Value::as_str) != Some(*source) {
            return Err(format!("raster source mapping changed: {section}/{size}").into());
        }
    }
    Ok(())
}

fn validate_raster_source_manifest(root: &Path) -> Result<()> {
    let path = root.join("branding/raster-sources.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&path)?)?;
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("raster source manifest schema changed".into());
    }
    validate_raster_source_section(&manifest, "windows", &WINDOWS_RASTER_SOURCES)?;
    validate_raster_source_section(&manifest, "toolbar", &TOOLBAR_RASTER_SOURCES)?;
    Ok(())
}

fn validate_svg(path: &Path, view_box: &str, ids: &[&str]) -> Result<(String, Document<'static>)> {
    let (text, document) = parse_svg(path)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(SVG_NS) || root.attribute("viewBox") != Some(view_box) {
        return Err(format!("{}: SVG namespace/viewBox invariant failed", path.display()).into());
    }
    if document.descendants().any(|node| {
        ["filter", "image", "linearGradient", "radialGradient"].contains(&node.tag_name().name())
    }) {
        return Err(format!(
            "{}: raster/filter/gradient element is forbidden",
            path.display()
        )
        .into());
    }
    for id in ids {
        if document
            .descendants()
            .all(|node| node.attribute("id") != Some(id))
        {
            return Err(format!("{}: missing element #{id}", path.display()).into());
        }
    }
    Ok((text, document))
}

fn validate_ico(root: &Path) -> Result<()> {
    let data = fs::read(root.join("branding/exports/windows/sky-auto-player.ico"))?;
    let directory = IconDir::read(Cursor::new(&data))?;
    if directory.resource_type() != ResourceType::Icon
        || directory.entries().len() != EXPECTED_SIZES.len()
    {
        return Err("branding ICO resource type/layer count changed".into());
    }
    let mut seen = BTreeMap::new();
    for (index, entry) in directory.entries().iter().enumerate() {
        let expected = ICO_ORDER[index];
        if entry.width() != expected
            || entry.height() != expected
            || !EXPECTED_SIZES.contains(&entry.width())
            || seen.insert(entry.width(), true).is_some()
        {
            return Err("branding ICO layer set is invalid".into());
        }
        if entry.is_png() != (expected == 256) {
            return Err("branding ICO encoding policy changed".into());
        }
        let image = entry.decode()?;
        if (image.width(), image.height()) != (expected, expected) {
            return Err("branding ICO decoded dimensions changed".into());
        }
    }
    Ok(())
}

pub fn validate(root: &Path) -> Result<()> {
    let branding = root.join("branding");
    validate_brand_lock(root)?;
    validate_raster_source_manifest(root)?;
    let ids = [
        "plate",
        "edge-a-b",
        "edge-a-c",
        "diamond-a",
        "node-b-gold",
        "node-c-sky",
    ];
    let (_, canonical) = validate_svg(
        &branding.join("sky-auto-player-app-icon.svg"),
        "0 0 128 128",
        &ids,
    )?;
    let (_, small) = validate_svg(
        &branding.join("sky-auto-player-app-icon-small.svg"),
        "0 0 128 128",
        &ids,
    )?;
    let (_, tiny) = validate_svg(
        &branding.join("sky-auto-player-app-icon-16.svg"),
        "0 0 128 128",
        &ids,
    )?;
    for document in [&small, &tiny, &canonical] {
        if document
            .descendants()
            .filter(|node| {
                node.attribute("id")
                    .is_some_and(|id| id.starts_with("dash-b-c-"))
            })
            .count()
            < 2
        {
            return Err("branding masters must contain their dashed edge".into());
        }
    }
    if canonical.descendants().any(|node| {
        node.attribute("id") == Some("plate") && node.attribute("fill") != Some("#07090D")
    }) {
        return Err("canonical branding plate color changed".into());
    }
    let toolbar = fs::read_to_string(root.join("desktop/src/components/shell/Toolbar.tsx"))?;
    for asset in [
        "app-icon-32.png",
        "app-icon-40.png",
        "app-icon-48.png",
        "app-icon-64.png",
    ] {
        if !toolbar.contains(asset) {
            return Err(format!("desktop toolbar is missing density asset {asset}").into());
        }
    }
    for (name, size) in [
        ("app-icon-32.png", 32),
        ("app-icon-40.png", 40),
        ("app-icon-48.png", 48),
        ("app-icon-64.png", 64),
    ] {
        let toolbar_bytes = fs::read(root.join("desktop/src/assets/brand").join(name))?;
        if png_dimensions(&toolbar_bytes)? != (size, size) {
            return Err(format!("desktop toolbar asset {name} dimensions changed").into());
        }
        let raster_bytes = fs::read(
            branding
                .join("exports/windows/raster")
                .join(format!("{size}.png")),
        )?;
        if toolbar_bytes != raster_bytes {
            return Err(format!("desktop toolbar asset drift: {name}").into());
        }
    }
    let no_bg = branding.join("sky-auto-player-mark-no-bg.svg");
    let (_, no_bg_doc) = validate_svg(
        &no_bg,
        "0 0 128 128",
        &[
            "edge-a-b",
            "edge-a-c",
            "diamond-a",
            "node-b-gold",
            "node-c-sky",
        ],
    )?;
    if no_bg_doc
        .descendants()
        .any(|node| node.attribute("id") == Some("plate"))
    {
        return Err("transparent branding mark must not contain a plate".into());
    }
    for name in [
        "sky-auto-player-mark-mono.svg",
        "sky-auto-player-mark-mono-dark.svg",
        "sky-auto-player-mark-mono-solid.svg",
        "lockup-horizontal.svg",
        "lockup-stacked.svg",
    ] {
        let path = branding.join(name);
        let (text, document) = validate_svg(
            &path,
            if name.starts_with("lockup-") {
                document_view_box(&path)?
            } else {
                "0 0 128 128"
            },
            &[],
        )?;
        if name.starts_with("lockup-")
            && (!text.contains("Sky Auto Player")
                || !text.contains("Play the sheet.")
                || !text.contains("Not the keyboard."))
        {
            return Err(format!("{name}: tagline missing").into());
        }
        if document.root_element().attribute("viewBox").is_none() {
            return Err(format!("{name}: viewBox missing").into());
        }
    }
    validate_ico(root)?;
    let ico = fs::read(branding.join("exports/windows/sky-auto-player.ico"))?;
    for consumer in [
        "site/public/favicon.ico",
        "desktop/src-tauri/icons/icon.ico",
    ] {
        if fs::read(root.join(consumer))? != ico {
            return Err(format!("branding ICO consumer drift: {consumer}").into());
        }
    }
    for (name, size) in [
        ("favicon-16x16.png", 16),
        ("favicon-32x32.png", 32),
        ("apple-touch-icon.png", 180),
    ] {
        let bytes = fs::read(branding.join("exports/web").join(name))?;
        if png_dimensions(&bytes)? != (size, size) {
            return Err(format!("{name}: PNG dimensions changed").into());
        }
        if size <= 32
            && bytes
                != fs::read(
                    branding
                        .join("exports/windows/raster")
                        .join(format!("{size}.png")),
                )?
        {
            return Err(format!("{name}: raster consumer drift").into());
        }
    }
    for (left, right) in [
        (
            "site/public/favicon.svg",
            "branding/sky-auto-player-app-icon-small.svg",
        ),
        (
            "site/public/assets/sky-auto-player-mark.svg",
            "branding/sky-auto-player-app-icon.svg",
        ),
        (
            "site/public/assets/sky-auto-player-mark-mono.svg",
            "branding/sky-auto-player-mark-mono.svg",
        ),
        (
            "site/public/assets/sky-auto-player-mark-no-bg.svg",
            "branding/sky-auto-player-mark-no-bg.svg",
        ),
    ] {
        if fs::read(root.join(left))? != fs::read(root.join(right))? {
            return Err(format!("branding consumer drift: {left}").into());
        }
    }
    println!("[xtask] branding checks: PASS");
    Ok(())
}

fn document_view_box(path: &Path) -> Result<&'static str> {
    let text = fs::read_to_string(path)?;
    if text.contains("viewBox=\"0 0 540 180\"") {
        Ok("0 0 540 180")
    } else if text.contains("viewBox=\"0 0 420 300\"") {
        Ok("0 0 420 300")
    } else {
        Err(format!("{}: unknown lockup viewBox", path.display()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(size: u32) -> Vec<u8> {
        let image =
            IconImage::from_rgba_data(size, size, vec![255; (size as usize) * (size as usize) * 4]);
        let mut data = Vec::new();
        image.write_png(&mut data).expect("PNG");
        data
    }

    #[test]
    fn ico_builder_preserves_layer_order_and_fields() {
        let layers = EXPECTED_SIZES
            .into_iter()
            .map(|size| (size, png(size)))
            .collect();
        let ico = build_ico(&layers).expect("ICO");
        let directory = IconDir::read(Cursor::new(&ico)).expect("ICO directory");
        assert_eq!(directory.entries().len(), EXPECTED_SIZES.len());
        assert_eq!(directory.entries()[0].width(), 32);
        assert!(!directory.entries()[0].is_png());
        assert_eq!(
            directory.entries().last().expect("256px entry").width(),
            256
        );
        assert!(directory.entries().last().expect("256px entry").is_png());
        for entry in directory.entries() {
            assert_eq!(
                entry.decode().expect("encoded entry").width(),
                entry.width()
            );
        }
    }

    #[test]
    fn current_branding_passes_validation() {
        validate(&crate::repo::root()).expect("branding assets");
    }

    #[test]
    fn brand_source_hash_is_stable_across_line_endings() {
        let lf = b"<svg>\n<path />\n</svg>\n";
        let crlf = b"<svg>\r\n<path />\r\n</svg>\r\n";
        let bare_cr = b"<svg>\r<path />\r</svg>\r";

        assert_eq!(
            sha256_hex(&normalized_brand_source_bytes(lf)),
            sha256_hex(&normalized_brand_source_bytes(crlf))
        );
        assert_eq!(
            sha256_hex(&normalized_brand_source_bytes(lf)),
            sha256_hex(&normalized_brand_source_bytes(bare_cr))
        );
        assert_eq!(normalized_brand_source_bytes(lf), lf);
    }
}
