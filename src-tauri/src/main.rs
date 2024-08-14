// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    slice::Iter,
};

use tauri::{api::dialog::blocking::FileDialogBuilder, AppHandle, Manager};

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
async fn convert(path: PathBuf, app: AppHandle) -> bool {
    println!("Path: {}", path.to_string_lossy());
    if let Err(e) = parse_file(path, app).await {
        eprintln!("Error: {}", e);
        return false;
    }
    true
}

#[tauri::command]
fn open_file(path: PathBuf) -> Result<(), String> {
    let res = std::process::Command::new("explorer")
        .args(["/select,", &path.to_string_lossy()]) // The comma after select is not a typo
        .spawn();

    if res.is_err() {
        return Err("Failed to open file in file explorer".to_string());
    }

    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![convert, open_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// the payload type must implement `Serialize` and `Clone`.
#[derive(Clone, serde::Serialize)]
struct Payload {
    path: String,
}

async fn parse_file(path: PathBuf, app: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let parts = tokenize_file(path.clone()).await?;
    tokio::spawn(async move {
        println!("Requesting save path");
        let name_res = path.file_name();
        if name_res.is_none() {
            eprintln!("Error: Could not get file name");
            return;
        }
        let name = name_res.unwrap().to_string_lossy();
        let name_res = &name.split('.').next();
        if name_res.is_none() {
            eprintln!("Error: Could not get file name base");
            return;
        }
        let name = name_res.unwrap();
        let save_path = FileDialogBuilder::new()
            .set_title("Pick Folder to Save Separated Files")
            .set_directory(path.parent().unwrap_or(&PathBuf::from("")))
            .set_file_name(format!("{}.gpx", name).as_str())
            .pick_folder();
        if let Some(path) = save_path {
            let res = write_files(parts, name, &path).await;
            if res.is_err() {
                eprintln!("Error: {}", res.unwrap_err());
                return;
            }
            if !res.unwrap() {
                return;
            }

            let _ = app.emit_all(
                "written",
                Payload {
                    path: format!("{}\\{}", path.display(), name),
                },
            );
        }
    });
    Ok(())
}

fn parse_indent_level(part: &str) -> i64 {
    let part = part.trim();
    if is_opening_tag(part) {
        1
    } else if is_closing_tag(part) {
        -1
    } else {
        0
    }
}

fn is_tag(part: &str) -> bool {
    is_opening_tag(part) || is_closing_tag(part) || is_self_closing_tag(part)
}

fn is_opening_tag(part: &str) -> bool {
    part.starts_with('<')
        && !part.starts_with("</")
        && !part.starts_with("<!")
        && !part.starts_with("<?")
        && !is_self_closing_tag(part)
}

fn is_closing_tag(part: &str) -> bool {
    part.starts_with("</")
}

fn is_self_closing_tag(part: &str) -> bool {
    (part.starts_with('<')
        && part.ends_with("/>")
        && !part.starts_with("</")
        && !part.starts_with("<!"))
        || (part.starts_with("<?") && part.ends_with("?>"))
}

async fn tokenize_file(path: PathBuf) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    println!("Tokenizing File: {}", path.display());
    let source = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(source);
    let iter = reader.split(b'<'); // Split on '<' to get tags
    let mut parts = vec![];
    // For each tag
    for line in iter {
        let mut line = line?;
        line.splice(..0, vec![b'<'].drain(..)); // Add back the '<' that was removed by split
        let line = String::from_utf8(line)?;
        // Split on '>' to get each element of the tag
        let part_iter = line
            .split_inclusive('>')
            .filter(|x| !x.is_empty() && !x.chars().all(char::is_whitespace));
        for part in part_iter {
            parts.push(part.to_owned());
        }
    }

    println!("Tokenized File");
    Ok(parts[1..].to_vec())
}

async fn write_files(
    parts: Vec<String>,
    base_name: &str,
    path: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    println!("Writing files");
    let wpt_path = PathBuf::from(format!("{}/{}_wpt.gpx", path.display(), base_name));
    let rte_path = PathBuf::from(format!("{}/{}_rte.gpx", path.display(), base_name));
    let trk_path = PathBuf::from(format!("{}/{}_trk.gpx", path.display(), base_name));

    let token_path = PathBuf::from(format!("{}/{}_tokens.txt", path.display(), base_name));

    let tkn_file = std::fs::File::create(token_path)?;
    let mut tkn_writer = std::io::BufWriter::new(tkn_file);
    for p in parts.iter() {
        tkn_writer.write_all(p.as_bytes())?;
        tkn_writer.write_all(b"\n")?;
    }

    // Ask to overwrite
    if wpt_path.exists() || rte_path.exists() || trk_path.exists() {
        let res = tauri::api::dialog::blocking::ask(
            Option::<&tauri::Window<tauri::Wry>>::None,
            "Overwrite?",
            "Files already exist, overwrite?",
        );
        if !res {
            return Ok(false);
        }
    }

    // Create files
    let wpt_file = std::fs::File::create(wpt_path)?;
    let rte_file = std::fs::File::create(rte_path)?;
    let trk_file = std::fs::File::create(trk_path)?;
    // Create writers for files
    let mut wpt_writer = std::io::BufWriter::new(wpt_file);
    let mut rte_writer = std::io::BufWriter::new(rte_file);
    let mut trk_writer = std::io::BufWriter::new(trk_file);
    // Read source file

    let mut last_was_val = true;
    let mut indent_level = 0;
    let mut last_indent_level = 0;

    let mut iter = parts.iter();
    while let Some(part) = iter.next() {
        let mut part = part.clone();

        if !handle_tag(
            "wpt",
            &mut wpt_writer,
            &mut part,
            &mut indent_level,
            &mut last_indent_level,
            &mut last_was_val,
            &mut iter,
        )
        .await
        .unwrap_or(false)
            && !handle_tag(
                "rte",
                &mut rte_writer,
                &mut part,
                &mut indent_level,
                &mut last_indent_level,
                &mut last_was_val,
                &mut iter,
            )
            .await
            .unwrap_or(false)
            && !handle_tag(
                "trk",
                &mut trk_writer,
                &mut part,
                &mut indent_level,
                &mut last_indent_level,
                &mut last_was_val,
                &mut iter,
            )
            .await
            .unwrap_or(false)
        {
            write_other_tags(
                &mut [&mut wpt_writer, &mut rte_writer, &mut trk_writer],
                &part,
                &mut indent_level,
                &mut last_indent_level,
                &mut last_was_val,
            )
            .await?;
        }
        println!("Files written successfully");
    }
    Ok(true)
}

async fn handle_tag(
    tag_name: &str,
    writer: &mut std::io::BufWriter<std::fs::File>,
    part: &mut str,
    indent_level: &mut i64,
    last_indent_level: &mut i64,
    last_was_val: &mut bool,
    iter: &mut Iter<'_, String>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut part = part.to_string();
    if part.trim().starts_with(format!("<{}", tag_name).as_str()) {
        loop {
            if is_self_closing_tag(part.as_str()) {
                writer.write_all(b"\n")?;
                for _ in 0..*indent_level {
                    writer.write_all(b"  ")?;
                }
                writer.write_all(part.as_bytes())?;
                if part.starts_with(format!("<{}", tag_name).as_str())
                    && part.ends_with("/>")
                    && !part
                        .chars()
                        .nth(tag_name.len() + 1)
                        .unwrap_or(' ')
                        .is_alphabetic()
                {
                    break;
                }
            }

            // Check if closing tag, as they need to process the indent level before writing the indent
            if is_closing_tag(part.trim()) {
                *last_indent_level = *indent_level;
                *indent_level += parse_indent_level(&part);
            }
            // write newline and indent if indent level changed and last was not a value (plain text)
            if indent_level != last_indent_level && !*last_was_val && is_tag(&part) {
                writer.write_all(b"\n")?;
                for _ in 0..*indent_level {
                    writer.write_all(b"  ")?;
                }
            }
            *last_was_val = !is_tag(&part);
            // Check if opening tag, as they need to process the indent level after writing the indent
            if is_opening_tag(part.trim()) {
                *last_indent_level = *indent_level;
                *indent_level += parse_indent_level(&part);
            }

            writer.write_all(part.as_bytes())?;
            if part.starts_with(format!("</{}>", tag_name).as_str()) {
                println!("Found closing tag: {}", part);
                break;
            }
            let next = iter.next();
            if let Some(next) = next {
                part = next.clone();
            } else {
                break;
            }
        }
        return Ok(true);
    }
    Ok(false)
}

async fn write_other_tags(
    writers: &mut [&mut std::io::BufWriter<std::fs::File>],
    part: &str,
    indent_level: &mut i64,
    last_indent_level: &mut i64,
    last_was_val: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if is_closing_tag(part.trim()) {
        println!("Found closing tag: {}", part);
        *last_indent_level = *indent_level;
        *indent_level += parse_indent_level(part);
    }

    for writer in writers.iter_mut() {
        if !(*last_was_val
            || !is_tag(part)
            || indent_level == last_indent_level && *indent_level != 0)
        {
            writer.write_all(b"\n")?;
            for _ in 0..*indent_level {
                writer.write_all(b"  ")?;
            }
        }

        if part.contains('\n') {
            let lines = part.split('\n');

            for line in lines {
                if line.ends_with('>') {
                    let self_closing = line.ends_with("/>");
                    writer.write_all(
                        line[..line.len() - if self_closing { 2 } else { 1 }].as_bytes(),
                    )?;

                    writer.write_all(b"\n")?;
                    for _ in 0..*indent_level {
                        writer.write_all(b"  ")?;
                    }
                    if self_closing {
                        writer.write_all(b"/>")?;
                    } else {
                        writer.write_all(b">")?;
                    }
                } else {
                    writer.write_all(line.as_bytes())?;
                    writer.write_all(b"\n")?;
                    for _ in 0..*indent_level + 1 {
                        writer.write_all(b"  ")?;
                    }
                }
            }
        } else {
            writer.write_all(part.as_bytes())?;
        }
    }
    *last_was_val = !is_tag(part);
    if is_opening_tag(part.trim()) {
        *last_indent_level = *indent_level;
        *indent_level += parse_indent_level(part);
    }
    Ok(())
}
