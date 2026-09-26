use super::{
    BrowserError, DiffHunk, DiffLine, DiffLineKind, MAX_HUNKS_PER_FILE, MAX_LINES_PER_HUNK,
};

pub(super) fn parse_hunks(source: &str) -> Result<(Vec<DiffHunk>, u64, u64, bool), BrowserError> {
    let mut hunks = Vec::new();
    let mut active = None;
    let mut additions = 0;
    let mut deletions = 0;
    let mut truncated = false;
    for line in source.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            if let Some(hunk) = active.take() {
                hunks.push(hunk);
            }
            if hunks.len() == MAX_HUNKS_PER_FILE {
                truncated = true;
                break;
            }
            active = Some(parse_hunk_header(header)?);
            continue;
        }
        let Some(hunk) = active.as_mut() else {
            continue;
        };
        if hunk.lines.len() == MAX_LINES_PER_HUNK {
            hunk.truncated = true;
            truncated = true;
            continue;
        }
        let (kind, text) = match line.as_bytes().first() {
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            Some(b'+') if !line.starts_with("+++") => {
                additions += 1;
                (DiffLineKind::Added, &line[1..])
            }
            Some(b'-') if !line.starts_with("---") => {
                deletions += 1;
                (DiffLineKind::Removed, &line[1..])
            }
            _ => continue,
        };
        let old_line = match kind {
            DiffLineKind::Added => None,
            DiffLineKind::Context | DiffLineKind::Removed => {
                Some(hunk.old_start + hunk.old_lines_seen)
            }
        };
        let new_line = match kind {
            DiffLineKind::Removed => None,
            DiffLineKind::Context | DiffLineKind::Added => {
                Some(hunk.new_start + hunk.new_lines_seen)
            }
        };
        if !matches!(kind, DiffLineKind::Added) {
            hunk.old_lines_seen += 1;
        }
        if !matches!(kind, DiffLineKind::Removed) {
            hunk.new_lines_seen += 1;
        }
        hunk.lines.push(DiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        });
    }
    if let Some(hunk) = active {
        hunks.push(hunk);
    }
    Ok((
        hunks
            .into_iter()
            .map(|hunk| DiffHunk {
                old_start: hunk.old_start,
                old_lines: hunk.old_lines,
                new_start: hunk.new_start,
                new_lines: hunk.new_lines,
                lines: hunk.lines,
                truncated: hunk.truncated,
            })
            .collect(),
        additions,
        deletions,
        truncated,
    ))
}

struct ActiveHunk {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    old_lines_seen: u32,
    new_lines_seen: u32,
    lines: Vec<DiffLine>,
    truncated: bool,
}

fn parse_hunk_header(value: &str) -> Result<ActiveHunk, BrowserError> {
    let (range, _context) = value.split_once(" @@").ok_or(BrowserError::Git)?;
    let mut ranges = range.split_whitespace();
    let old = parse_hunk_range(ranges.next().ok_or(BrowserError::Git)?, '-')?;
    let new = parse_hunk_range(ranges.next().ok_or(BrowserError::Git)?, '+')?;
    Ok(ActiveHunk {
        old_start: old.0,
        old_lines: old.1,
        new_start: new.0,
        new_lines: new.1,
        old_lines_seen: 0,
        new_lines_seen: 0,
        lines: Vec::new(),
        truncated: false,
    })
}

fn parse_hunk_range(value: &str, prefix: char) -> Result<(u32, u32), BrowserError> {
    let value = value.strip_prefix(prefix).ok_or(BrowserError::Git)?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Ok((
        start.parse().map_err(|_| BrowserError::Git)?,
        count.parse().map_err(|_| BrowserError::Git)?,
    ))
}

pub(super) fn truncate_text(mut value: String, maximum: usize) -> String {
    if value.len() > maximum {
        value.truncate(maximum);
    }
    value
}

pub(super) fn validate_object_id(value: &str) -> Result<(), BrowserError> {
    if (value.len() == 40 || value.len() == 64)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument)
    }
}
