//! MCP (stdio JSON-RPC) for Buzz room DocSmith specialist.
//!
//! Tools:
//! - pdf_inspect / pdf_create
//! - docx_read / docx_create
//! - xlsx_read / xlsx_create
//! - pptx_create

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use docx_rs::{Docx, Paragraph, Run, Table, TableCell, TableRow};
use printpdf::{BuiltinFont, Mm, PdfDocument};
use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn main() {
    if let Err(e) = run() {
        eprintln!("bony-docs-tools-mcp fatal: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let line = line.context("read stdin")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("skip bad json: {e}");
                continue;
            }
        };
        if let Some(resp) = handle_message(&req) {
            let out = serde_json::to_string(&resp)?;
            writeln!(stdout, "{out}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle_message(req: &Value) -> Option<Value> {
    let method = req.get("method")?.as_str()?;
    let id = req.get("id").cloned();
    if id.is_none() && method.starts_with("notifications/") {
        return None;
    }

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": req
                .pointer("/params/protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2024-11-05")),
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "bony-docs-tools-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => {
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Ok(call_tool(name, &args))
        }
        other => Err(rpc_error(-32601, format!("Method not found: {other}"))),
    };

    match (id, result) {
        (Some(id), Ok(result)) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })),
        (Some(id), Err(err)) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": err
        })),
        _ => None,
    }
}

fn rpc_error(code: i64, message: String) -> Value {
    json!({ "code": code, "message": message })
}

fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "name": "pdf_inspect",
            "description": "Inspect a PDF: page count + extracted text sample. Classifies text-heavy vs mostly-empty/scanned-like heuristically (inspired by pdf-inspector routing).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to .pdf file" },
                    "max_chars": { "type": "integer", "description": "Max text chars to return (default 8000)" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "pdf_create",
            "description": "Create a simple multi-page PDF from plain text or markdown-like lines. Output path ends with .pdf.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Output .pdf path" },
                    "title": { "type": "string" },
                    "body": { "type": "string", "description": "Full document body; blank lines become paragraph breaks" }
                },
                "required": ["path", "body"]
            }
        }),
        json!({
            "name": "docx_read",
            "description": "Extract plain text from a .docx file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_chars": { "type": "integer" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "docx_create",
            "description": "Create a .docx with title + paragraphs and optional simple table rows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "title": { "type": "string" },
                    "paragraphs": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "table_rows": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "description": "Optional table: array of rows (each row array of cell strings)"
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "xlsx_read",
            "description": "Read spreadsheet (.xlsx/.xls/.ods) via calamine. Returns sheet names and cell grid as TSV per sheet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_rows": { "type": "integer", "description": "Max rows per sheet (default 200)" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "xlsx_create",
            "description": "Create .xlsx workbook. sheets: [{name, rows: [[cell,...],...]}].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "sheets": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "rows": {
                                    "type": "array",
                                    "items": {
                                        "type": "array",
                                        "items": { "type": "string" }
                                    }
                                }
                            },
                            "required": ["name", "rows"]
                        }
                    }
                },
                "required": ["path", "sheets"]
            }
        }),
        json!({
            "name": "pptx_create",
            "description": "Create a minimal .pptx with one text slide per string in slides[]. Basic layout only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "title": { "type": "string", "description": "Presentation title (first slide header if slides empty)" },
                    "slides": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Slide body text (one slide each)"
                    }
                },
                "required": ["path"]
            }
        }),
    ]
}

fn call_tool(name: &str, args: &Value) -> Value {
    let result = match name {
        "pdf_inspect" => tool_pdf_inspect(args),
        "pdf_create" => tool_pdf_create(args),
        "docx_read" => tool_docx_read(args),
        "docx_create" => tool_docx_create(args),
        "xlsx_read" => tool_xlsx_read(args),
        "xlsx_create" => tool_xlsx_create(args),
        "pptx_create" => tool_pptx_create(args),
        other => Err(format!("unknown tool: {other}")),
    };
    match result {
        Ok(text) => json!({
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }),
        Err(err) => json!({
            "content": [{ "type": "text", "text": format!("Error: {err}") }],
            "isError": true
        }),
    }
}

fn arg_path(args: &Value) -> Result<PathBuf, String> {
    args.get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "path is required".into())
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    Ok(())
}

fn tool_pdf_inspect(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000) as usize;

    let bytes = fs::read(&path).map_err(|e| format!("read: {e}"))?;
    let doc = pdf_oxide::PdfDocument::from_bytes(bytes)
        .map_err(|e| format!("pdf open failed: {e}"))?;
    let pages = doc
        .page_count()
        .map_err(|e| format!("page_count: {e}"))?;
    let mut text = String::new();
    for i in 0..pages {
        if text.len() >= max_chars {
            break;
        }
        match doc.extract_text(i) {
            Ok(page_text) if !page_text.trim().is_empty() => {
                text.push_str(&format!("--- page {} ---\n{}\n", i + 1, page_text));
            }
            Ok(_) => {}
            Err(e) => {
                text.push_str(&format!("--- page {} ---\n[extract error: {e}]\n", i + 1));
            }
        }
    }
    let chars = text.chars().count();
    let per_page = if pages > 0 {
        chars as f64 / pages as f64
    } else {
        0.0
    };
    // Heuristic similar to pdf-inspector routing: very low text density ⇒ scanned-like.
    let kind = if pages == 0 {
        "empty"
    } else if per_page < 40.0 {
        "scanned_or_image_heavy"
    } else if per_page < 200.0 {
        "mixed_or_sparse_text"
    } else {
        "text_based"
    };
    let sample: String = text.chars().take(max_chars).collect();
    Ok(format!(
        "path={}\npages={pages}\nclassification={kind}\nchars_extracted={chars}\ntext_per_page≈{per_page:.1}\n\n{sample}",
        path.display()
    ))
}

fn tool_pdf_create(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "body is required".to_string())?;
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Document");
    ensure_parent(&path)?;

    let (doc, page1, layer1) = PdfDocument::new(title, Mm(210.0), Mm(297.0), "Layer 1");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| format!("font: {e}"))?;

    let lines: Vec<&str> = body.lines().collect();
    let mut page_idx = 0usize;
    let mut current_layer = doc.get_page(page1).get_layer(layer1);
    let mut y = 280.0_f32;

    // Title on first page
    current_layer.use_text(title, 16.0, Mm(20.0), Mm(y), &font);
    y -= 12.0;

    for line in lines {
        if y < 20.0 {
            page_idx += 1;
            let (page, layer) = doc.add_page(Mm(210.0), Mm(297.0), format!("Page {}", page_idx + 1));
            current_layer = doc.get_page(page).get_layer(layer);
            y = 280.0;
        }
        let clip: String = line.chars().take(95).collect();
        current_layer.use_text(&clip, 11.0, Mm(20.0), Mm(y), &font);
        y -= 6.0;
        if line.trim().is_empty() {
            y -= 3.0;
        }
    }

    use std::io::BufWriter;
    let file = File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|e| format!("save pdf: {e}"))?;
    Ok(format!("wrote {}", path.display()))
}

fn tool_docx_read(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(12000) as usize;
    // docx is a zip; pull word/document.xml text nodes roughly via zip + string scan.
    let file = File::open(&path).map_err(|e| format!("open: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip: {e}"))?;
    let mut xml = String::new();
    {
        let mut doc = archive
            .by_name("word/document.xml")
            .map_err(|e| format!("document.xml: {e}"))?;
        doc.read_to_string(&mut xml)
            .map_err(|e| format!("read xml: {e}"))?;
    }
    let text = strip_xml_text(&xml);
    let sample: String = text.chars().take(max_chars).collect();
    Ok(format!(
        "path={}\nchars={}\n\n{sample}",
        path.display(),
        text.chars().count()
    ))
}

fn strip_xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_was_space = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space {
                        out.push(' ');
                        last_was_space = true;
                    }
                } else {
                    out.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tool_docx_create(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    ensure_parent(&path)?;
    let title = args.get("title").and_then(|v| v.as_str());
    let paragraphs: Vec<String> = args
        .get("paragraphs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut doc = Docx::new();
    if let Some(t) = title {
        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(t).size(28).bold()),
        );
    }
    for p in &paragraphs {
        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(p)));
    }
    if let Some(rows) = args.get("table_rows").and_then(|v| v.as_array()) {
        let mut table = Table::new(vec![]);
        for row in rows {
            let cells: Vec<TableCell> = row
                .as_array()
                .map(|cols| {
                    cols.iter()
                        .map(|c| {
                            let s = c.as_str().unwrap_or("");
                            TableCell::new().add_paragraph(
                                Paragraph::new().add_run(Run::new().add_text(s)),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !cells.is_empty() {
                table = table.add_row(TableRow::new(cells));
            }
        }
        doc = doc.add_table(table);
    }
    let file = File::create(&path).map_err(|e| format!("create: {e}"))?;
    doc.build().pack(file).map_err(|e| format!("docx pack: {e}"))?;
    Ok(format!("wrote {}", path.display()))
}

fn tool_xlsx_read(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let max_rows = args
        .get("max_rows")
        .and_then(|v| v.as_u64())
        .unwrap_or(200) as usize;
    let mut workbook = open_workbook_auto(&path).map_err(|e| format!("open workbook: {e}"))?;
    let sheets = workbook.sheet_names().to_vec();
    let mut out = format!("path={}\nsheets={}\n", path.display(), sheets.join(", "));
    for name in sheets {
        out.push_str(&format!("\n## sheet: {name}\n"));
        if let Ok(range) = workbook.worksheet_range(&name) {
            let mut row_i = 0usize;
            for row in range.rows() {
                if row_i >= max_rows {
                    out.push_str("…(truncated rows)\n");
                    break;
                }
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| match c {
                        Data::Empty => String::new(),
                        other => other.to_string(),
                    })
                    .collect();
                out.push_str(&cells.join("\t"));
                out.push('\n');
                row_i += 1;
            }
        }
    }
    const MAX: usize = 48_000;
    if out.len() > MAX {
        out = format!("…[truncated]…\n{}", &out[out.len() - MAX..]);
    }
    Ok(out)
}

fn tool_xlsx_create(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    ensure_parent(&path)?;
    let sheets = args
        .get("sheets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "sheets array required".to_string())?;
    let mut workbook = Workbook::new();
    for sheet in sheets {
        let name = sheet
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Sheet1");
        let worksheet = workbook.add_worksheet();
        worksheet
            .set_name(name)
            .map_err(|e| format!("set name: {e}"))?;
        let rows = sheet
            .get("rows")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for (r, row) in rows.iter().enumerate() {
            let cols = row.as_array().cloned().unwrap_or_default();
            for (c, cell) in cols.iter().enumerate() {
                let s = cell.as_str().unwrap_or("");
                worksheet
                    .write_string(r as u32, c as u16, s)
                    .map_err(|e| format!("write cell: {e}"))?;
            }
        }
    }
    workbook
        .save(&path)
        .map_err(|e| format!("save xlsx: {e}"))?;
    Ok(format!("wrote {}", path.display()))
}

fn tool_pptx_create(args: &Value) -> Result<String, String> {
    let path = arg_path(args)?;
    ensure_parent(&path)?;
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Presentation");
    let mut slides: Vec<String> = args
        .get("slides")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if slides.is_empty() {
        slides.push(title.to_string());
    }

    let file = File::create(&path).map_err(|e| format!("create: {e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("[Content_Types].xml", opts)
        .map_err(|e| e.to_string())?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
  <Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
"#
    )
    .map_err(|e| e.to_string())?;
    for i in 1..=slides.len() {
        write!(
            zip,
            r#"  <Override PartName="/ppt/slides/slide{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
"#
        )
        .map_err(|e| e.to_string())?;
    }
    write!(zip, "</Types>").map_err(|e| e.to_string())?;

    zip.start_file("_rels/.rels", opts)
        .map_err(|e| e.to_string())?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
    )
    .map_err(|e| e.to_string())?;

    // presentation.xml
    zip.start_file("ppt/presentation.xml", opts)
        .map_err(|e| e.to_string())?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst>
"#
    )
    .map_err(|e| e.to_string())?;
    for i in 1..=slides.len() {
        write!(
            zip,
            r#"    <p:sldId id="{}" r:id="rId{}"/>
"#,
            255 + i as u32,
            i
        )
        .map_err(|e| e.to_string())?;
    }
    write!(
        zip,
        r#"  </p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
  <p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>"#
    )
    .map_err(|e| e.to_string())?;

    zip.start_file("ppt/_rels/presentation.xml.rels", opts)
        .map_err(|e| e.to_string())?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
"#
    )
    .map_err(|e| e.to_string())?;
    for i in 1..=slides.len() {
        write!(
            zip,
            r#"  <Relationship Id="rId{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{i}.xml"/>
"#
        )
        .map_err(|e| e.to_string())?;
    }
    let after = slides.len() + 1;
    write!(
        zip,
        r#"  <Relationship Id="rId{after}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>
</Relationships>"#,
        after + 1
    )
    .map_err(|e| e.to_string())?;

    for (i, body) in slides.iter().enumerate() {
        let n = i + 1;
        let escaped = xml_escape(body);
        zip.start_file(format!("ppt/slides/slide{n}.xml"), opts)
            .map_err(|e| e.to_string())?;
        write!(
            zip,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="457200" y="274320"/><a:ext cx="11277600" cy="914400"/></a:xfrm>
          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
        </p:spPr>
        <p:txBody>
          <a:bodyPr/><a:lstStyle/>
          <a:p><a:r><a:rPr lang="en-US" sz="2800" b="1"/><a:t>{title}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="457200" y="1371600"/><a:ext cx="11277600" cy="4572000"/></a:xfrm>
          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
        </p:spPr>
        <p:txBody>
          <a:bodyPr/><a:lstStyle/>
          <a:p><a:r><a:rPr lang="en-US" sz="1800"/><a:t>{escaped}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>"#,
            title = xml_escape(title),
            escaped = escaped
        )
        .map_err(|e| e.to_string())?;

        zip.start_file(format!("ppt/slides/_rels/slide{n}.xml.rels"), opts)
            .map_err(|e| e.to_string())?;
        write!(
            zip,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#
        )
        .map_err(|e| e.to_string())?;
    }

    // Minimal master / layout / theme stubs
    for (name, body) in [
        (
            "ppt/slideMasters/slideMaster1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
  </p:spTree></p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>
</p:sldMaster>"#,
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#,
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1">
  <p:cSld name="Blank"><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
  </p:spTree></p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>"#,
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#,
        ),
        (
            "ppt/theme/theme1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="1F497D"/></a:dk2>
      <a:lt2><a:srgbClr val="EEECE1"/></a:lt2>
      <a:accent1><a:srgbClr val="4F81BD"/></a:accent1>
      <a:accent2><a:srgbClr val="C0504D"/></a:accent2>
      <a:accent3><a:srgbClr val="9BBB59"/></a:accent3>
      <a:accent4><a:srgbClr val="8064A2"/></a:accent4>
      <a:accent5><a:srgbClr val="4BACC6"/></a:accent5>
      <a:accent6><a:srgbClr val="F79646"/></a:accent6>
      <a:hlink><a:srgbClr val="0000FF"/></a:hlink>
      <a:folHlink><a:srgbClr val="800080"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst>
      <a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>
      <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
      <a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#,
        ),
    ] {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| format!("zip finish: {e}"))?;
    Ok(format!(
        "wrote {} ({} slides)",
        path.display(),
        slides.len()
    ))
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c if c.is_control() && c != '\n' && c != '\t' => String::new(),
            c => c.to_string(),
        })
        .collect()
}
