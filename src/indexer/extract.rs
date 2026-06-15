use crate::error::{Result, ServerError};
use std::path::Path;

/// Extract text from a file, handling PDFs via `pdf_oxide` and plain text via UTF-8 read.
pub fn extract_text(path: &Path) -> Result<String> {
    if is_pdf(path) {
        extract_pdf_text(path)
    } else {
        std::fs::read_to_string(path).map_err(|e| {
            ServerError::DatabaseError(format!("failed to read {}: {}", path.display(), e))
        })
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let mut doc = pdf_oxide::api::Pdf::open(path).map_err(|e| {
        ServerError::DatabaseError(format!("failed to open PDF {}: {}", path.display(), e))
    })?;

    let page_count = doc.page_count().map_err(|e| {
        ServerError::DatabaseError(format!(
            "failed to get page count for {}: {}",
            path.display(),
            e
        ))
    })?;

    let mut full_text = String::new();
    let mut failed_pages = Vec::new();
    for page in 0..page_count {
        match doc.to_text(page) {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if !full_text.is_empty() {
                        full_text.push_str("\n\n");
                    }
                    full_text.push_str(trimmed);
                }
            }
            Err(_) => {
                failed_pages.push(page + 1);
            }
        }
    }

    if !failed_pages.is_empty() && full_text.is_empty() {
        return Err(ServerError::DatabaseError(format!(
            "PDF extraction failed for all pages in {} (failed pages: {:?})",
            path.display(),
            failed_pages
        )));
    }

    Ok(full_text)
}
