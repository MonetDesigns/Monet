//! API coverage: how much of `@figma/plugin-typings` this layer actually implements.
//!
//! "1:1 with Figma" is not a switch. It is a number that has to be measured, and kept honest as
//! Figma's API moves. The workflow this module supports:
//!
//! 1. The typings' `plugin-api.d.ts` is parsed (outside this crate; it is a TypeScript file) into a
//!    list of members: `area<TAB>member<TAB>status<TAB>note`.
//! 2. [`Coverage::parse_tsv`] reads that list, counts it, and refuses a status it doesn't know.
//! 3. [`Coverage::markdown`] writes the report a PR and Plugins ▸ Manage plugins can show: the
//!    percentage per area, and every unsupported member with the reason it is unsupported.
//!
//! A member is `implemented` (the proxy and the engine can do it), `unsupported` (deliberately not
//! provided yet, with a note that reaches the user), or `missing` (planned, not written yet).
//! Nothing is silently absent: a member missing from the list is a coverage bug in the generator,
//! not a supported API.

use std::collections::BTreeMap;

/// What a member's row says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Implemented,
    Unsupported,
    Missing,
}

impl Status {
    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "implemented" | "ok" => Some(Self::Implemented),
            "unsupported" | "no" => Some(Self::Unsupported),
            "missing" | "todo" | "pending" => Some(Self::Missing),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Unsupported => "unsupported",
            Self::Missing => "missing",
        }
    }
}

/// One API member and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// `Figma`, `SceneNode`, `TextNode`, `Variables`, `UI`, …
    pub area: String,
    /// `createRectangle`, `layoutMode`, `showUI`, …
    pub name: String,
    pub status: Status,
    /// For `unsupported`: why, in a sentence a user can read. Empty otherwise.
    pub note: String,
}

/// The parsed list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    members: Vec<Member>,
}

/// The counts, without allocating.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub total: usize,
    pub implemented: usize,
    pub unsupported: usize,
    pub missing: usize,
}

impl Report {
    /// Implemented members as a percentage of all members (0 when the list is empty).
    pub fn percent(&self) -> f32 {
        if self.total == 0 { 0.0 } else { self.implemented as f32 / self.total as f32 * 100.0 }
    }
}

/// Why a coverage list can't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoverageError {
    #[error("line {line}: expected `area<TAB>member<TAB>status<TAB>note`")]
    Columns { line: usize },
    #[error("line {line}: `{status}` isn't implemented, unsupported or missing")]
    Status { line: usize, status: String },
    #[error("line {line}: `{field}` is empty")]
    Empty { line: usize, field: &'static str },
}

impl Coverage {
    /// Read a generated list. `#` starts a comment, blank lines are skipped, and three columns
    /// (with no note) are accepted.
    pub fn parse_tsv(text: &str) -> Result<Self, CoverageError> {
        let mut members = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let raw = raw.trim_end();
            if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
                continue;
            }
            let mut columns = raw.split('\t');
            let area = columns.next().unwrap_or_default().trim();
            let name = columns.next().unwrap_or_default().trim();
            let status = columns.next().unwrap_or_default().trim();
            let note = columns.next().unwrap_or_default().trim();
            if !columns.next().is_none() {
                return Err(CoverageError::Columns { line });
            }
            if status.is_empty() {
                return Err(CoverageError::Columns { line });
            }
            if area.is_empty() {
                return Err(CoverageError::Empty { line, field: "area" });
            }
            if name.is_empty() {
                return Err(CoverageError::Empty { line, field: "member" });
            }
            let Some(status) = Status::parse(status) else {
                return Err(CoverageError::Status { line, status: status.into() });
            };
            members.push(Member { area: area.into(), name: name.into(), status, note: note.into() });
        }
        Ok(Self { members })
    }

    /// Add one member, for a hand-written starter list.
    pub fn push(&mut self, member: Member) {
        self.members.push(member);
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The counts.
    pub fn report(&self) -> Report {
        let mut report = Report { total: self.members.len(), ..Report::default() };
        for member in &self.members {
            match member.status {
                Status::Implemented => report.implemented += 1,
                Status::Unsupported => report.unsupported += 1,
                Status::Missing => report.missing += 1,
            }
        }
        report
    }

    /// The Markdown report: totals, a row per area, and every unsupported member with its reason.
    pub fn markdown(&self) -> String {
        let report = self.report();
        let mut out = String::new();
        out.push_str("# Figma plugin API coverage\n\n");
        out.push_str(&format!(
            "{} members: {} implemented ({:.0}%), {} unsupported, {} missing.\n\n",
            report.total,
            report.implemented,
            report.percent(),
            report.unsupported,
            report.missing
        ));
        out.push_str("| Area | Members | Implemented | Unsupported | Missing | Coverage |\n|---|---|---|---|---|---|\n");
        let mut areas: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
        for member in &self.members {
            let row = areas.entry(member.area.as_str()).or_default();
            match member.status {
                Status::Implemented => row[0] += 1,
                Status::Unsupported => row[1] += 1,
                Status::Missing => row[2] += 1,
            }
        }
        for (area, counts) in areas {
            let total = counts[0] + counts[1] + counts[2];
            let percent = if total == 0 { 0.0 } else { counts[0] as f32 / total as f32 * 100.0 };
            out.push_str(&format!("| {area} | {total} | {} | {} | {} | {percent:.0}% |\n", counts[0], counts[1], counts[2]));
        }
        let unsupported: Vec<&Member> = self.members.iter().filter(|member| member.status == Status::Unsupported).collect();
        if !unsupported.is_empty() {
            out.push_str("\n## Unsupported, and why\n\n");
            for member in unsupported {
                let note = if member.note.is_empty() { "not provided yet" } else { member.note.as_str() };
                out.push_str(&format!("- `{}.{}`: {note}\n", member.area, member.name));
            }
        }
        out
    }

    /// A line per member, in the input format, with counts kept: what a generator writes and a
    /// later diff reads.
    pub fn to_tsv(&self) -> String {
        let mut out = String::new();
        for member in &self.members {
            out.push_str(&format!("{}\t{}\t{}\t{}\n", member.area, member.name, member.status.label(), member.note));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Coverage, CoverageError, Member, Report, Status};

    const SAMPLE: &str = "# area\tmember\tstatus\tnote\nFigma\tcreateRectangle\timplemented\t\nFigma\tshowUI\tmissing\t\nTextNode\tcharacters\tunsupported\tneeds loadFontAsync\t\nSceneNode\tlayoutMode\timplemented\t\n";

    #[test]
    fn reads_comments_and_three_columns() {
        let coverage = Coverage::parse_tsv(SAMPLE).unwrap();
        assert_eq!(coverage.members().len(), 4);
        assert_eq!(coverage.members()[2].note, "needs loadFontAsync");
        let three = "Figma\tnotify\timplemented\n";
        assert_eq!(Coverage::parse_tsv(three).unwrap().members()[0].status, Status::Implemented);
    }

    #[test]
    fn refuses_a_bad_row() {
        assert_eq!(Coverage::parse_tsv("Figma\tx\tdone\n"), Err(CoverageError::Status { line: 1, status: "done".into() }));
        assert_eq!(Coverage::parse_tsv("Figma\tx\n"), Err(CoverageError::Columns { line: 1 }));
        assert_eq!(Coverage::parse_tsv("Figma\tx\timplemented\ttoo\tmany\n"), Err(CoverageError::Columns { line: 1 }));
    }

    #[test]
    fn counts_and_percentages() {
        let coverage = Coverage::parse_tsv(SAMPLE).unwrap();
        assert_eq!(coverage.report(), Report { total: 4, implemented: 2, unsupported: 1, missing: 1 });
        assert_eq!(coverage.report().percent(), 50.0);
        let markdown = coverage.markdown();
        assert!(markdown.contains("50%"), "{markdown}");
        assert!(markdown.contains("`TextNode.characters`: needs loadFontAsync"), "{markdown}");
        assert!(markdown.contains("| Figma | 2 | 1 | 0 | 1 | 50% |"), "{markdown}");
    }

    #[test]
    fn round_trips_through_tsv() {
        let mut coverage = Coverage::default();
        coverage.push(Member { area: "Figma".into(), name: "createPage".into(), status: Status::Missing, note: String::new() });
        let text = coverage.to_tsv();
        let again = Coverage::parse_tsv(&text).unwrap();
        assert_eq!(again.members(), coverage.members());
    }
}
