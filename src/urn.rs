/// smem URN — a stable, machine-readable source identifier for any fact stored
/// in semantic memory, regardless of where it originated.
///
/// # Format
///
/// ```text
/// urn:smem:<type>:<origin>:<locator>[#<fragment>]
/// ```
///
/// Every component is lowercase ASCII except `<locator>` and `<fragment>`,
/// which preserve the case of the underlying system (paths, URLs, etc.).
///
/// # Content types
///
/// | Type   | Meaning                                          |
/// |--------|--------------------------------------------------|
/// | `code` | Source code file                                 |
/// | `doc`  | Documentation, markdown, prose                   |
/// | `web`  | Live web content                                 |
/// | `data` | Structured data — DB rows, API payloads, CSV     |
/// | `note` | Manually authored or synthesized fact            |
/// | `conf` | Configuration file                               |
///
/// # Origins and locator shapes
///
/// | Origin   | Locator shape                                           | Example locator                                          |
/// |----------|---------------------------------------------------------|----------------------------------------------------------|
/// | `git`    | `<host>:<org>:<repo>:<branch>:<path>`                   | `github.com:acme:repo:main:src/lib.rs`                  |
/// | `git`    | `<host>::<repo>:<branch>:<path>`                       | `git.example.com::repo:main:src/lib.rs` (no org)        |
/// | `fs`     | `[<hostname>:]<absolute-path>`                          | `/home/user/project/src/main.rs` or `my-nas:/mnt/share/file.txt` |
/// | `https`  | `<host>/<path>`                                         | `docs.example.com/api/overview`                          |
/// | `http`   | `<host>/<path>`                                         | `intranet.corp/wiki/setup`                               |
/// | `db`     | `<driver>/<host>/<database>/<table>/<pk>`               | `postgres/localhost/myapp/users/abc-123`                 |
/// | `api`    | `<host>/<path>`                                         | `api.example.com/v2/facts/abc-123`                       |
/// | `manual` | `<label>`                                               | `2026-03-04/onboarding-session`                          |
///
/// # Fragment (`#`)
///
/// | Shape      | Meaning                                              |
/// |------------|------------------------------------------------------|
/// | `L42`      | Single line 42                                       |
/// | `L10-L30`  | Lines 10 through 30                                  |
/// | `<slug>`   | Section anchor (HTML `id` or Markdown heading slug)  |
///
/// # Full examples
///
/// ```text
/// urn:smem:code:git:github.com/acme/repo/refs/heads/main/src/lib.rs#L1-L50
/// urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/main.rs#L10
/// urn:smem:doc:https:docs.anthropic.com/mcp/protocol#tools
/// urn:smem:doc:fs:/Users/sienna/Development/sunbeam/README.md
/// urn:smem:data:db:postgres/localhost/sunbeam/facts/abc-123
/// urn:smem:note:manual:2026-03-04/onboarding-session
/// urn:smem:conf:fs:/etc/myapp/config.toml
/// ```

use serde_json::{json, Value};

// ── constants ─────────────────────────────────────────────────────────────────

const PREFIX: &str = "urn:smem:";

/// Machine-readable spec string embedded into MCP tool descriptions so any
/// client can understand the format without out-of-band documentation.
pub const SPEC: &str = "\
smem URN format:  urn:smem:<type>:<origin>:<locator>[#<fragment>]

Content types:  code doc web data note conf
Origins:        git fs https http db api manual

Origin locator shapes:
  git    <host>:<org>:<repo>:<branch>:<path>  (ARN-like format, org optional)
  fs     [<hostname>:]<absolute-path>  (hostname optional; identifies NAS, remote machine, cloud drive, etc.)
  https  <host>/<path>
  http   <host>/<path>
  db     <driver>/<host>/<database>/<table>/<pk>
  api    <host>/<path>
  manual <label>

Fragment (#):
  L42      single line
  L10-L30  line range
  <slug>   section anchor / HTML id

Examples:
  urn:smem:code:git:github.com:acme:repo:main:src/lib.rs#L1-L50
  urn:smem:code:git:git.example.com::repo:main:src/lib.rs
  urn:smem:code:fs:/home/user/project/src/main.rs#L10
  urn:smem:doc:https:docs.example.com/api/overview#authentication
  urn:smem:data:db:postgres/localhost/mydb/users/abc-123
  urn:smem:note:manual:2026-03-04/session-notes";

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ContentType {
    Code,
    Doc,
    Web,
    Data,
    Note,
    Conf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    Git,
    Fs,
    Https,
    Http,
    Db,
    Api,
    Manual,
}

#[derive(Debug, Clone)]
pub struct SourceUrn {
    pub content_type: ContentType,
    pub origin: Origin,
    /// Origin-specific locator string; see the per-origin shapes above.
    pub locator: String,
    /// Optional sub-location within the source (line range, anchor, etc.).
    pub fragment: Option<String>,
}

#[derive(Debug)]
pub struct UrnError(pub String);

impl std::fmt::Display for UrnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── parsing ───────────────────────────────────────────────────────────────────

impl ContentType {
    fn parse(s: &str) -> Result<Self, UrnError> {
        match s {
            "code" => Ok(Self::Code),
            "doc"  => Ok(Self::Doc),
            "web"  => Ok(Self::Web),
            "data" => Ok(Self::Data),
            "note" => Ok(Self::Note),
            "conf" => Ok(Self::Conf),
            other  => Err(UrnError(format!(
                "unknown content type {other:?}; valid: code, doc, web, data, note, conf"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Doc  => "doc",
            Self::Web  => "web",
            Self::Data => "data",
            Self::Note => "note",
            Self::Conf => "conf",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Code => "source code",
            Self::Doc  => "documentation",
            Self::Web  => "web page",
            Self::Data => "data record",
            Self::Note => "note",
            Self::Conf => "configuration",
        }
    }
}

impl Origin {
    fn parse(s: &str) -> Result<Self, UrnError> {
        match s {
            "git"    => Ok(Self::Git),
            "fs"     => Ok(Self::Fs),
            "https"  => Ok(Self::Https),
            "http"   => Ok(Self::Http),
            "db"     => Ok(Self::Db),
            "api"    => Ok(Self::Api),
            "manual" => Ok(Self::Manual),
            other    => Err(UrnError(format!(
                "unknown origin {other:?}; valid: git, fs, https, http, db, api, manual"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Git    => "git",
            Self::Fs     => "fs",
            Self::Https  => "https",
            Self::Http   => "http",
            Self::Db     => "db",
            Self::Api    => "api",
            Self::Manual => "manual",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Git    => "git repository",
            Self::Fs     => "local file",
            Self::Https | Self::Http => "web URL",
            Self::Db     => "database",
            Self::Api    => "API endpoint",
            Self::Manual => "manually authored",
        }
    }
}

// ── SourceUrn ─────────────────────────────────────────────────────────────────

impl SourceUrn {
    /// Build and validate a URN from string components.
    ///
    /// Validates each component and returns the canonical URN string, or a
    /// `UrnError` if any component is invalid.
    pub fn build(
        content_type_str: &str,
        origin_str: &str,
        locator: &str,
        fragment: Option<&str>,
    ) -> Result<String, UrnError> {
        if locator.is_empty() {
            return Err(UrnError("locator must not be empty".to_string()));
        }
        if let Some(f) = fragment {
            if f.is_empty() {
                return Err(UrnError("fragment must not be empty if provided".to_string()));
            }
        }
        let urn = Self {
            content_type: ContentType::parse(content_type_str)?,
            origin: Origin::parse(origin_str)?,
            locator: locator.to_string(),
            fragment: fragment.map(|f| f.to_string()),
        };
        Ok(urn.to_urn())
    }

    /// Parse a `urn:smem:...` string into its components.
    ///
    /// Returns `UrnError` with a human-readable message on any malformed input.
    pub fn parse(input: &str) -> Result<Self, UrnError> {
        let rest = input.strip_prefix(PREFIX).ok_or_else(|| {
            UrnError(format!("must start with '{PREFIX}'; got {input:?}"))
        })?;

        // Fragment splits on the first '#'; the fragment may itself contain '#'.
        let (body, fragment) = match rest.split_once('#') {
            Some((b, f)) if f.is_empty() => {
                return Err(UrnError("fragment after '#' must not be empty".to_string()));
            }
            Some((b, f)) => (b, Some(f.to_string())),
            None => (rest, None),
        };

        // body = <type>:<origin>:<locator>
        // splitn(3) so that the locator can itself contain colons (e.g. fs paths, db URIs).
        let mut parts = body.splitn(3, ':');
        let type_str   = parts.next().filter(|s| !s.is_empty())
            .ok_or_else(|| UrnError("missing <type> component".to_string()))?;
        let origin_str = parts.next().filter(|s| !s.is_empty())
            .ok_or_else(|| UrnError("missing <origin> component".to_string()))?;
        let locator    = parts.next().filter(|s| !s.is_empty())
            .ok_or_else(|| UrnError("missing <locator> component".to_string()))?;

        Ok(Self {
            content_type: ContentType::parse(type_str)?,
            origin: Origin::parse(origin_str)?,
            locator: locator.to_string(),
            fragment,
        })
    }

    /// Reconstitute the canonical URN string.
    pub fn to_urn(&self) -> String {
        let frag = self.fragment.as_deref()
            .map(|f| format!("#{f}"))
            .unwrap_or_default();
        format!(
            "{}{}:{}:{}{}",
            PREFIX, self.content_type.as_str(), self.origin.as_str(), self.locator, frag
        )
    }

    /// Return structured JSON describing every parsed component.
    /// Used by the `parse_source_urn` MCP tool.
    pub fn describe(&self) -> Value {
        json!({
            "valid": true,
            "content_type": self.content_type.as_str(),
            "origin": self.origin.as_str(),
            "locator": self.locator,
            "fragment": self.fragment,
            "human_readable": self.human_readable(),
        })
    }

    pub fn human_readable(&self) -> String {
        let frag = match &self.fragment {
            None    => String::new(),
            Some(f) if f.starts_with('L') => format!(" ({})", f.replace('-', "–")),
            Some(f) => format!(" §{f}"),
        };
        format!(
            "{} from {}: {}{}",
            self.content_type.label(), self.origin.label(), self.locator, frag
        )
    }

    //─── Git-specific Methods (ARN-like format) ────────────────────────────────

    /// Validate that a git URN has the correct ARN-like format
    pub fn is_valid_git_urn(&self) -> bool {
        if self.origin != Origin::Git {
            return false;
        }
        
        // Split by colon to parse ARN-like format
        let parts: Vec<&str> = self.locator.split(':').collect();
        
        // Minimum valid format: host::repo:branch:path (5 parts)
        // Or: host:org:repo:branch:path (6+ parts)
        if parts.len() < 5 {
            return false;
        }
        
        // Validate required components are not empty
        // parts[0] = host, parts[2] = repo, parts[3] = branch, parts[4..] = path
        parts[0].is_empty() == false && 
        parts[2].is_empty() == false && 
        parts[3].is_empty() == false && 
        parts[4..].join(":").is_empty() == false
    }

    /// Extract host from git locator (ARN-like format: host:org:repo:branch:path)
    pub fn extract_git_host(&self) -> Option<&str> {
        if !self.is_valid_git_urn() {
            return None;
        }
        
        let parts: Vec<&str> = self.locator.split(':').collect();
        Some(parts[0])
    }

    /// Extract organization from git locator (returns None if no org)
    pub fn extract_git_org(&self) -> Option<&str> {
        if !self.is_valid_git_urn() {
            return None;
        }
        
        let parts: Vec<&str> = self.locator.split(':').collect();
        
        // parts[1] is org field - empty string means no org
        if parts[1].is_empty() {
            None
        } else {
            Some(parts[1])
        }
    }

    /// Extract repository name from git locator
    pub fn extract_git_repo(&self) -> Option<&str> {
        if !self.is_valid_git_urn() {
            return None;
        }
        
        let parts: Vec<&str> = self.locator.split(':').collect();
        
        // parts[2] is always the repo name
        Some(parts[2])
    }

    /// Extract branch from git locator
    pub fn extract_git_branch(&self) -> Option<&str> {
        if !self.is_valid_git_urn() {
            return None;
        }
        
        let parts: Vec<&str> = self.locator.split(':').collect();
        
        // parts[3] is always the branch name
        Some(parts[3])
    }

    /// Extract file path from git locator (preserves / separators)
    pub fn extract_git_path(&self) -> Option<String> {
        if !self.is_valid_git_urn() {
            return None;
        }
        
        let parts: Vec<&str> = self.locator.split(':').collect();
        
        // parts[4..] is the path - join with : to preserve any colons in the path
        // This handles paths like "src:lib.rs" (though rare)
        Some(parts[4..].join(":"))
    }

    /// Build a git URN with ARN-like format: host:org:repo:branch:path
    pub fn build_git_urn(
        host: &str,
        org: Option<&str>,
        repo: &str,
        branch: &str,
        path: &str,
        fragment: Option<&str>
    ) -> Result<String, UrnError> {
        // Validate required components
        if host.is_empty() {
            return Err(UrnError("host cannot be empty".to_string()));
        }
        if repo.is_empty() {
            return Err(UrnError("repo cannot be empty".to_string()));
        }
        if branch.is_empty() {
            return Err(UrnError("branch cannot be empty".to_string()));
        }
        if path.is_empty() {
            return Err(UrnError("path cannot be empty".to_string()));
        }
        
        // Build locator: host:org:repo:branch:path
        let org_part = org.unwrap_or("");
        let locator = format!("{}:{}:{}:{}:{}", host, org_part, repo, branch, path);
        
        // Build full URN
        let fragment_part = fragment.map(|f| format!("#{}", f)).unwrap_or_default();
        Ok(format!("urn:smem:code:git:{}{}", locator, fragment_part))
    }
}

// ── schema ────────────────────────────────────────────────────────────────────

/// Return machine-readable taxonomy of all valid URN components.
/// Used by the `describe_urn_schema` MCP tool.
pub fn schema_json() -> Value {
    json!({
        "format": "urn:smem:<type>:<origin>:<locator>[#<fragment>]",
        "content_types": [
            { "value": "code", "label": "source code" },
            { "value": "doc",  "label": "documentation" },
            { "value": "web",  "label": "web page" },
            { "value": "data", "label": "data record" },
            { "value": "note", "label": "note" },
            { "value": "conf", "label": "configuration" }
        ],
        "origins": [
            {
                "value": "git",
                "label": "git repository",
                "locator_shape": "<host>:<org>:<repo>:<branch>:<path>",
                "note": "ARN-like format with colons. Org is optional (use :: for no org). Path preserves / separators.",
                "examples": [
                    "github.com:acme:repo:main:src/lib.rs",
                    "git.example.com::repo:main:src/lib.rs",
                    "github.com:acme:repo:feature/new-auth:src/auth.rs"
                ]
            },
            {
                "value": "fs",
                "label": "local or remote file",
                "locator_shape": "[<hostname>:]<absolute-path>",
                "note": "Hostname is optional. Omit for local files; include to identify a NAS, remote machine, cloud drive mount, etc.",
                "examples": [
                    "/home/user/project/src/main.rs",
                    "my-nas:/mnt/share/docs/readme.txt",
                    "macbook-pro:/Users/sienna/notes.md",
                    "google-drive:/My Drive/design.gdoc"
                ]
            },
            {
                "value": "https",
                "label": "web URL (https)",
                "locator_shape": "<host>/<path>",
                "example": "docs.example.com/api/overview"
            },
            {
                "value": "http",
                "label": "web URL (http)",
                "locator_shape": "<host>/<path>",
                "example": "intranet.corp/wiki/setup"
            },
            {
                "value": "db",
                "label": "database record",
                "locator_shape": "<driver>/<host>/<database>/<table>/<pk>",
                "example": "postgres/localhost/myapp/users/abc-123"
            },
            {
                "value": "api",
                "label": "API endpoint",
                "locator_shape": "<host>/<path>",
                "example": "api.example.com/v2/facts/abc-123"
            },
            {
                "value": "manual",
                "label": "manually authored",
                "locator_shape": "<label>",
                "example": "2026-03-04/onboarding-session"
            }
        ],
        "fragment_shapes": [
            { "pattern": "L42",     "meaning": "single line 42" },
            { "pattern": "L10-L30", "meaning": "lines 10 through 30" },
            { "pattern": "<slug>",  "meaning": "section anchor / HTML id" }
        ],
        "examples": [
            "urn:smem:code:git:github.com:acme:repo:main:src/lib.rs#L1-L50",
            "urn:smem:code:git:git.example.com::repo:main:src/lib.rs",
            "urn:smem:code:fs:/home/user/project/src/main.rs#L10",
            "urn:smem:code:fs:my-nas:/mnt/share/src/main.rs",
            "urn:smem:doc:https:docs.example.com/api/overview#authentication",
            "urn:smem:data:db:postgres/localhost/mydb/users/abc-123",
            "urn:smem:note:manual:2026-03-04/session-notes"
        ]
    })
}

// ── error response helper ─────────────────────────────────────────────────────

/// Build the JSON response for an invalid URN (used by the MCP tool).
pub fn invalid_urn_response(input: &str, err: &UrnError) -> Value {
    json!({
        "valid": false,
        "input": input,
        "error": err.to_string(),
        "spec": SPEC,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_git() {
        let s = "urn:smem:code:git:github.com/acme/repo/refs/heads/main/src/lib.rs#L1-L50";
        let urn = SourceUrn::parse(s).unwrap();
        assert_eq!(urn.content_type, ContentType::Code);
        assert_eq!(urn.origin, Origin::Git);
        assert_eq!(urn.locator, "github.com/acme/repo/refs/heads/main/src/lib.rs");
        assert_eq!(urn.fragment.as_deref(), Some("L1-L50"));
        assert_eq!(urn.to_urn(), s);
    }

    #[test]
    fn roundtrip_fs_no_fragment() {
        let s = "urn:smem:doc:fs:/Users/sienna/README.md";
        let urn = SourceUrn::parse(s).unwrap();
        assert_eq!(urn.origin, Origin::Fs);
        assert!(urn.fragment.is_none());
        assert_eq!(urn.to_urn(), s);
    }

    #[test]
    fn roundtrip_db_with_colons_in_locator() {
        // locator contains slashes but the split is on ':' — fs paths have ':' on Windows
        // and db locators may too; splitn(3) ensures the locator is taken verbatim.
        let s = "urn:smem:data:db:postgres/localhost/mydb/users/abc-123";
        let urn = SourceUrn::parse(s).unwrap();
        assert_eq!(urn.locator, "postgres/localhost/mydb/users/abc-123");
        assert_eq!(urn.to_urn(), s);
    }

    #[test]
    fn roundtrip_https_with_anchor() {
        let s = "urn:smem:doc:https:docs.example.com/api/overview#authentication";
        let urn = SourceUrn::parse(s).unwrap();
        assert_eq!(urn.origin, Origin::Https);
        assert_eq!(urn.fragment.as_deref(), Some("authentication"));
        assert_eq!(urn.to_urn(), s);
    }

    #[test]
    fn err_missing_prefix() {
        assert!(SourceUrn::parse("smem:code:fs:/foo").is_err());
    }

    #[test]
    fn err_unknown_type() {
        assert!(SourceUrn::parse("urn:smem:blob:fs:/foo").is_err());
    }

    #[test]
    fn err_unknown_origin() {
        assert!(SourceUrn::parse("urn:smem:code:ftp:/foo").is_err());
    }

    #[test]
    fn err_empty_fragment() {
        assert!(SourceUrn::parse("urn:smem:code:fs:/foo#").is_err());
    }

    #[test]
    fn build_valid_urn() {
        let urn = SourceUrn::build("code", "fs", "/Users/sienna/file.rs", Some("L10-L30")).unwrap();
        assert_eq!(urn, "urn:smem:code:fs:/Users/sienna/file.rs#L10-L30");
    }

    #[test]
    fn build_valid_fs_with_hostname() {
        let urn = SourceUrn::build("doc", "fs", "my-nas:/mnt/share/readme.txt", None).unwrap();
        assert_eq!(urn, "urn:smem:doc:fs:my-nas:/mnt/share/readme.txt");
        // Verify it round-trips through parse
        let parsed = SourceUrn::parse(&urn).unwrap();
        assert_eq!(parsed.locator, "my-nas:/mnt/share/readme.txt");
    }

    #[test]
    fn build_err_empty_locator() {
        assert!(SourceUrn::build("code", "fs", "", None).is_err());
    }

    #[test]
    fn build_err_empty_fragment() {
        assert!(SourceUrn::build("code", "fs", "/foo", Some("")).is_err());
    }

    #[test]
    fn schema_json_has_required_fields() {
        let s = schema_json();
        assert!(s["content_types"].is_array());
        assert!(s["origins"].is_array());
        assert!(s["fragment_shapes"].is_array());
        assert!(s["examples"].is_array());
        // fs origin should describe hostname support
        let fs = s["origins"].as_array().unwrap()
            .iter()
            .find(|o| o["value"] == "fs")
            .expect("fs origin");
        assert!(fs["note"].as_str().unwrap().contains("NAS") ||
                fs["locator_shape"].as_str().unwrap().contains("hostname"));
    }

    #[test]
    fn human_readable_line_range() {
        let urn = SourceUrn::parse(
            "urn:smem:code:git:github.com/acme/repo/refs/heads/main/src/lib.rs#L10-L30"
        ).unwrap();
        let desc = urn.human_readable();
        assert!(desc.contains("source code"));
        assert!(desc.contains("git repository"));
        assert!(desc.contains("L10–L30"));
    }

    //─── New ARN-like Git URN Tests ─────────────────────────────────────────

    #[test]
    fn test_git_urn_basic_parsing() {
        // Test that the new format can be parsed at all
        let urn_str = "urn:smem:code:git:github.com:acme:repo:main:src/lib.rs#L1-L50";
        let urn = SourceUrn::parse(urn_str).unwrap();
        
        // Verify basic parsing works
        assert_eq!(urn.content_type, ContentType::Code);
        assert_eq!(urn.origin, Origin::Git);
        assert_eq!(urn.locator, "github.com:acme:repo:main:src/lib.rs");
        assert_eq!(urn.fragment.as_deref(), Some("L1-L50"));
        
        // Verify round-trip works
        assert_eq!(urn.to_urn(), urn_str);
    }

    #[test]
    fn test_git_urn_no_org_parsing() {
        // Test parsing without organization (double colon)
        let urn_str = "urn:smem:code:git:git.example.com::repo:main:src/lib.rs";
        let urn = SourceUrn::parse(urn_str).unwrap();
        
        assert_eq!(urn.content_type, ContentType::Code);
        assert_eq!(urn.origin, Origin::Git);
        assert_eq!(urn.locator, "git.example.com::repo:main:src/lib.rs");
        assert_eq!(urn.to_urn(), urn_str);
    }

    #[test]
    fn test_git_urn_basic_format() {
        // Standard format with all components
        let urn_str = "urn:smem:code:git:github.com:acme:repo:main:src/lib.rs#L1-L50";
        let urn = SourceUrn::parse(urn_str).unwrap();
        
        // Verify basic parsing
        assert_eq!(urn.content_type, ContentType::Code);
        assert_eq!(urn.origin, Origin::Git);
        assert_eq!(urn.locator, "github.com:acme:repo:main:src/lib.rs");
        assert_eq!(urn.fragment.as_deref(), Some("L1-L50"));
        
        // Verify round-trip
        assert_eq!(urn.to_urn(), urn_str);
    }

    #[test]
    fn test_git_urn_with_organization() {
        let urn = SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:main:src/lib.rs").unwrap();
        assert_eq!(urn.extract_git_org(), Some("acme"));
    }

    #[test]
    fn test_git_urn_without_organization() {
        // Empty org field (double colon)
        let urn = SourceUrn::parse("urn:smem:code:git:git.example.com::repo:main:src/lib.rs").unwrap();
        assert_eq!(urn.extract_git_org(), None);
    }

    #[test]
    fn test_git_urn_component_extraction() {
        let urn = SourceUrn::parse("urn:smem:code:git:gitlab.com:company:project:dev:src/main.rs").unwrap();
        
        assert_eq!(urn.extract_git_host(), Some("gitlab.com"));
        assert_eq!(urn.extract_git_org(), Some("company"));
        assert_eq!(urn.extract_git_repo(), Some("project"));
        assert_eq!(urn.extract_git_branch(), Some("dev"));
        assert_eq!(urn.extract_git_path(), Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_git_urn_path_preservation() {
        // Paths should preserve their natural / separators
        let urn = SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:main:src/components/Button.tsx").unwrap();
        assert_eq!(urn.extract_git_path(), Some("src/components/Button.tsx".to_string()));
    }

    #[test]
    fn test_git_urn_branch_with_slashes() {
        // Feature branches often have slashes
        let urn = SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:feature/new-auth:src/auth.rs").unwrap();
        assert_eq!(urn.extract_git_branch(), Some("feature/new-auth"));
    }

    #[test]
    fn test_git_urn_validation_valid_cases() {
        // All valid formats
        assert!(SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:main:src/lib.rs").is_ok());
        assert!(SourceUrn::parse("urn:smem:code:git:git.example.com::repo:main:src/lib.rs").is_ok());
        assert!(SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:feature/branch:src/file.rs").is_ok());
    }

    #[test]
    fn test_git_urn_validation_invalid_cases() {
        // Missing components - these should parse but be invalid
        let urn1 = SourceUrn::parse("urn:smem:code:git:github.com:acme:repo:main").unwrap(); // Missing path
        assert!(!urn1.is_valid_git_urn());
        
        let urn2 = SourceUrn::parse("urn:smem:code:git:github.com:acme:repo").unwrap(); // Missing branch and path
        assert!(!urn2.is_valid_git_urn());
        
        let urn3 = SourceUrn::parse("urn:smem:code:git:github.com:acme").unwrap(); // Missing repo, branch, path
        assert!(!urn3.is_valid_git_urn());
        
        // Empty required components
        let urn4 = SourceUrn::parse("urn:smem:code:git::acme:repo:main:src/lib.rs").unwrap(); // Empty host
        assert!(!urn4.is_valid_git_urn());
        
        let urn5 = SourceUrn::parse("urn:smem:code:git:github.com::::src/lib.rs").unwrap(); // Empty repo and branch
        assert!(!urn5.is_valid_git_urn());
    }

    #[test]
    fn test_git_urn_roundtrip_consistency() {
        // Parse -> Serialize -> Parse should be identical
        let original = "urn:smem:code:git:gitlab.com:company:project:feature/new-ui:src/components/Button.tsx#L42";
        
        let parsed1 = SourceUrn::parse(original).unwrap();
        let serialized = parsed1.to_urn();
        let parsed2 = SourceUrn::parse(&serialized).unwrap();
        
        assert_eq!(parsed1.locator, parsed2.locator);
        assert_eq!(parsed1.fragment, parsed2.fragment);
        assert_eq!(serialized, original);
    }

    #[test]
    fn test_build_git_urn_with_organization() {
        let urn = SourceUrn::build_git_urn(
            "github.com",
            Some("acme"),
            "repo",
            "main",
            "src/lib.rs",
            Some("L10-L20")
        ).unwrap();
        
        assert_eq!(urn, "urn:smem:code:git:github.com:acme:repo:main:src/lib.rs#L10-L20");
    }

    #[test]
    fn test_build_git_urn_without_organization() {
        let urn = SourceUrn::build_git_urn(
            "git.example.com",
            None,
            "repo",
            "main",
            "src/lib.rs",
            None
        ).unwrap();
        
        assert_eq!(urn, "urn:smem:code:git:git.example.com::repo:main:src/lib.rs");
    }

    #[test]
    fn test_build_git_urn_validation() {
        // Empty host
        assert!(SourceUrn::build_git_urn("", Some("acme"), "repo", "main", "src/lib.rs", None).is_err());
        
        // Empty repo
        assert!(SourceUrn::build_git_urn("github.com", Some("acme"), "", "main", "src/lib.rs", None).is_err());
        
        // Empty branch
        assert!(SourceUrn::build_git_urn("github.com", Some("acme"), "repo", "", "src/lib.rs", None).is_err());
        
        // Empty path
        assert!(SourceUrn::build_git_urn("github.com", Some("acme"), "repo", "main", "", None).is_err());
    }
}
