use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;

use super::Tool;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Web search via Brave Search API.
pub struct WebSearchTool {
    api_key: String,
    max_results: u32,
}

impl WebSearchTool {
    pub fn new(api_key: String, max_results: u32) -> Self {
        Self {
            api_key,
            max_results,
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "count": {"type": "integer", "description": "Number of results (1-10)", "minimum": 1, "maximum": 10}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let query = params
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string();

        if query.is_empty() {
            return Ok("Error: query is required".into());
        }

        if self.api_key.is_empty() {
            return Ok("Error: BRAVE_API_KEY not configured. Set tools.web.search.apiKey in config.json or BRAVE_API_KEY env var.".into());
        }

        let count = params
            .get("count")
            .and_then(|c| c.as_u64())
            .map(|c| c.min(10) as u32)
            .unwrap_or(self.max_results);

        let client = reqwest::Client::new();
        let resp = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", &query), ("count", &count.to_string())])
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        resp.error_for_status_ref()
            .map_err(|e| anyhow::anyhow!("Brave Search API error: {e}"))?;

        let body: serde_json::Value = resp.json().await?;
        let results = body
            .get("web")
            .and_then(|w| w.get("results"))
            .and_then(|r| r.as_array());

        let results = match results {
            Some(r) if !r.is_empty() => r,
            _ => return Ok(format!("No results for: {query}")),
        };

        let mut output = format!("Results for: {query}\n");
        for (i, result) in results.iter().enumerate() {
            let title = result.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let desc = result
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            output.push_str(&format!("\n{}. {title}\n   {url}\n   {desc}", i + 1));
        }

        Ok(output)
    }
}

/// Fetch a URL and extract readable content.
pub struct WebFetchTool {
    max_chars: usize,
    tag_re: Regex,
    script_re: Regex,
    style_re: Regex,
    link_re: Regex,
    heading_re: Regex,
    li_re: Regex,
    block_close_re: Regex,
    br_re: Regex,
    spaces_re: Regex,
    newlines_re: Regex,
}

impl WebFetchTool {
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars,
            tag_re: Regex::new(r"<[^>]+>").unwrap(),
            script_re: Regex::new(r"(?is)<script[\s\S]*?</script>").unwrap(),
            style_re: Regex::new(r"(?is)<style[\s\S]*?</style>").unwrap(),
            link_re: Regex::new(r#"(?is)<a\s+[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)</a>"#)
                .unwrap(),
            heading_re: Regex::new(r"(?is)<h([1-6])[^>]*>([\s\S]*?)</h[1-6]>").unwrap(),
            li_re: Regex::new(r"(?is)<li[^>]*>([\s\S]*?)</li>").unwrap(),
            block_close_re: Regex::new(r"(?i)</(p|div|section|article)>").unwrap(),
            br_re: Regex::new(r"(?i)<(br|hr)\s*/?>").unwrap(),
            spaces_re: Regex::new(r"[ \t]+").unwrap(),
            newlines_re: Regex::new(r"\n{3,}").unwrap(),
        }
    }

    fn strip_tags(&self, html: &str) -> String {
        let text = self.script_re.replace_all(html, "");
        let text = self.style_re.replace_all(&text, "");
        let text = self.tag_re.replace_all(&text, "");
        html_escape::decode_html_entities(&text).to_string()
    }

    fn to_markdown(&self, html: &str) -> String {
        // Convert links
        let text = self.link_re.replace_all(html, |caps: &regex::Captures| {
            let url = &caps[1];
            let inner = self.strip_tags(&caps[2]);
            format!("[{inner}]({url})")
        });

        // Convert headings
        let text = self
            .heading_re
            .replace_all(&text, |caps: &regex::Captures| {
                let level: usize = caps[1].parse().unwrap_or(1);
                let inner = self.strip_tags(&caps[2]);
                format!("\n{} {inner}\n", "#".repeat(level))
            });

        // Convert list items
        let text = self.li_re.replace_all(&text, |caps: &regex::Captures| {
            let inner = self.strip_tags(&caps[1]);
            format!("\n- {inner}")
        });

        // Convert block elements
        let text = self.block_close_re.replace_all(&text, "\n\n");
        let text = self.br_re.replace_all(&text, "\n");

        // Strip remaining tags and normalize
        let text = self.strip_tags(&text);
        let text = self.spaces_re.replace_all(&text, " ");
        self.newlines_re
            .replace_all(&text, "\n\n")
            .trim()
            .to_string()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL and extract readable content (HTML to markdown/text)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to fetch"},
                "extractMode": {"type": "string", "enum": ["markdown", "text"], "default": "markdown"},
                "maxChars": {"type": "integer", "minimum": 100}
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let url = params
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        if url.is_empty() {
            return Ok("Error: url is required".into());
        }

        // Validate URL scheme
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "error": format!("Only http/https allowed, got '{}'", url.split(':').next().unwrap_or("none")),
                "url": url
            }))?);
        }

        let extract_mode = params
            .get("extractMode")
            .and_then(|m| m.as_str())
            .unwrap_or("markdown");

        let max_chars = params
            .get("maxChars")
            .and_then(|m| m.as_u64())
            .map(|m| m as usize)
            .unwrap_or(self.max_chars);

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()?;

        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "error": e.to_string(),
                    "url": url
                }))?);
            }
        };

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "error": e.to_string(),
                    "url": url
                }))?);
            }
        };

        let (text, extractor) = if content_type.contains("application/json") {
            // JSON content — pretty-print
            let formatted = match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(body),
                Err(_) => body,
            };
            (formatted, "json")
        } else if content_type.contains("text/html")
            || body.trim_start()[..body.len().min(256)]
                .to_lowercase()
                .starts_with("<!doctype")
            || body.trim_start()[..body.len().min(256)]
                .to_lowercase()
                .starts_with("<html")
        {
            // HTML content — extract with readability, fall back to regex
            let parsed_url = url::Url::parse(&final_url)
                .unwrap_or_else(|_| url::Url::parse("http://localhost").unwrap());
            let readability_result =
                readability::extractor::extract(&mut body.as_bytes(), &parsed_url);
            match readability_result {
                Ok(product) if !product.text.trim().is_empty() => {
                    let text = if extract_mode == "text" {
                        product.text
                    } else {
                        // Use extracted HTML content and convert to markdown
                        self.to_markdown(&product.content)
                    };
                    (text, "readability")
                }
                _ => {
                    // Fallback to regex-based extraction
                    let text = if extract_mode == "text" {
                        self.strip_tags(&body)
                    } else {
                        self.to_markdown(&body)
                    };
                    (text, "regex")
                }
            }
        } else {
            // Raw text
            (body, "raw")
        };

        let truncated = text.len() > max_chars;
        let text = if truncated {
            text[..max_chars].to_string()
        } else {
            text
        };

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "url": url,
            "finalUrl": final_url,
            "status": status,
            "extractor": extractor,
            "truncated": truncated,
            "length": text.len(),
            "text": text
        }))?)
    }
}
