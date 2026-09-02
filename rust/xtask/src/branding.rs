use crate::Result;
use roxmltree::Document;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const EXPECTED_SIZES: [u32; 14] = [
    16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 96, 128, 256,
];
const LARGE_SIZES: [u32; 8] = [32, 48, 60, 64, 72, 96, 128, 256];
const SMALL_SIZES: [u32; 5] = [20, 24, 30, 36, 40];
const TINY_SIZES: [u32; 1] = [16];
const SVG_NS: &str = "http://www.w3.org/2000/svg";

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
    let header_size = 6 + EXPECTED_SIZES.len() * 16;
    let mut entries = Vec::with_capacity(EXPECTED_SIZES.len() * 16);
    let mut payload = Vec::new();
    let mut offset = header_size as u32;
    for size in EXPECTED_SIZES {
        let image = layers.get(&size).expect("checked above");
        entries.extend_from_slice(&[
            if size == 256 { 0 } else { size as u8 },
            if size == 256 { 0 } else { size as u8 },
            0,
            0,
        ]);
        entries.extend_from_slice(&1u16.to_le_bytes());
        entries.extend_from_slice(&32u16.to_le_bytes());
        entries.extend_from_slice(&(image.len() as u32).to_le_bytes());
        entries.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(image);
        offset = offset
            .checked_add(image.len() as u32)
            .ok_or("ICO payload is too large")?;
    }
    let mut output = Vec::with_capacity(header_size + payload.len());
    output.extend_from_slice(&[0, 0, 1, 0]);
    output.extend_from_slice(&(EXPECTED_SIZES.len() as u16).to_le_bytes());
    output.extend_from_slice(&entries);
    output.extend_from_slice(&payload);
    Ok(output)
}

pub fn build_ico_from_dirs(large: &Path, small: &Path, tiny: &Path) -> Result<Vec<u8>> {
    let mut layers = find_layers(large, &LARGE_SIZES)?;
    layers.extend(find_layers(small, &SMALL_SIZES)?);
    layers.extend(find_layers(tiny, &TINY_SIZES)?);
    build_ico(&layers)
}

pub fn write_ico(large: &Path, small: &Path, tiny: &Path, output: &Path) -> Result<()> {
    let bytes = build_ico_from_dirs(large, small, tiny)?;
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
    if data.len() < 6
        || data[..4] != [0, 0, 1, 0]
        || u16::from_le_bytes(data[4..6].try_into()?) != EXPECTED_SIZES.len() as u16
    {
        return Err("branding ICO header/layer count changed".into());
    }
    let mut seen = BTreeMap::new();
    for index in 0..EXPECTED_SIZES.len() {
        let offset = 6 + index * 16;
        let width = if data[offset] == 0 {
            256
        } else {
            data[offset] as u32
        };
        let height = if data[offset + 1] == 0 {
            256
        } else {
            data[offset + 1] as u32
        };
        if width != height || !EXPECTED_SIZES.contains(&width) || seen.insert(width, true).is_some()
        {
            return Err("branding ICO layer set is invalid".into());
        }
        let size = u32::from_le_bytes(data[offset + 8..offset + 12].try_into()?) as usize;
        let start = u32::from_le_bytes(data[offset + 12..offset + 16].try_into()?) as usize;
        let layer = data
            .get(start..start + size)
            .ok_or("branding ICO layer is truncated")?;
        if png_dimensions(layer)? != (width, height) {
            return Err("branding ICO PNG dimensions changed".into());
        }
    }
    Ok(())
}

pub fn validate(root: &Path) -> Result<()> {
    let branding = root.join("branding");
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
        "0 0 24 24",
        &ids,
    )?;
    let (_, tiny) = validate_svg(
        &branding.join("sky-auto-player-app-icon-16.svg"),
        "0 0 16 16",
        &ids,
    )?;
    let mut optical_documents = vec![small, tiny];
    for (name, view_box) in [
        ("sky-auto-player-app-icon-20.svg", "0 0 20 20"),
        ("sky-auto-player-app-icon-24.svg", "0 0 24 24"),
        ("sky-auto-player-app-icon-30.svg", "0 0 30 30"),
        ("sky-auto-player-app-icon-32.svg", "0 0 32 32"),
        ("sky-auto-player-app-icon-36.svg", "0 0 36 36"),
        ("sky-auto-player-app-icon-40.svg", "0 0 40 40"),
        ("sky-auto-player-app-icon-48.svg", "0 0 48 48"),
    ] {
        let (_, document) = validate_svg(&branding.join(name), view_box, &ids)?;
        optical_documents.push(document);
    }
    for document in optical_documents.iter().chain(std::iter::once(&canonical)) {
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
    if !toolbar.contains("sky-auto-player-app-icon-32.svg") {
        return Err("desktop toolbar must use the dedicated 32px branding master".into());
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
    for consumer in ["site/public/favicon.ico", "desktop/src-tauri/icons/icon.ico"] {
        if fs::read(root.join(consumer))? != ico {
            return Err(format!("branding ICO consumer drift: {consumer}").into());
        }
    }
    for (name, size) in [
        ("favicon-16x16.png", 16),
        ("favicon-32x32.png", 32),
        ("apple-touch-icon.png", 180),
    ] {
        if png_dimensions(&fs::read(branding.join("exports/web").join(name))?)? != (size, size) {
            return Err(format!("{name}: PNG dimensions changed").into());
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
        let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(&size.to_be_bytes());
        data
    }

    #[test]
    fn ico_builder_preserves_layer_order_and_fields() {
        let layers = EXPECTED_SIZES
            .into_iter()
            .map(|size| (size, png(size)))
            .collect();
        let ico = build_ico(&layers).expect("ICO");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, EXPECTED_SIZES.len() as u8, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[6 + 16 * (EXPECTED_SIZES.len() - 1)], 0);
        assert_eq!(&ico[10..12], &1u16.to_le_bytes());
        assert_eq!(&ico[12..14], &32u16.to_le_bytes());
    }

    #[test]
    fn current_branding_passes_validation() {
        validate(&crate::repo::root()).expect("branding assets");
    }
}
