// Ferramentas que o MODELO pode chamar durante o chat (tool calls da API
// OpenAI-compat do llama-server). O backend executa; o front orquestra o
// loop (ver send() no main.ts).
//
// SEGURANÇA em primeiro lugar — o modelo gera os parâmetros, então tudo é
// tratado como entrada não-confiável:
//   1. formato: allowlist estrita (md, txt, html; docx entra na fatia C);
//   2. nome de arquivo: SEM componentes de path (sem / \ ..), sem chars
//      inválidos do Windows, sem pontos/espaços nas pontas, ≤ 100 chars;
//   3. confinamento: o arquivo cai SEMPRE dentro da pasta escolhida pelo
//      usuário (o join com nome sanitizado não pode escapar);
//   4. nunca sobrescreve: sufixo -1..-99, depois erro;
//   5. limite de tamanho do conteúdo (2 MB).

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Limite do conteúdo gerado pelo modelo (2 MB — um documento de texto é
/// ~500k tokens; maior que isso é modelo em loop).
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
/// Extensões permitidas por formato (allowlist).
const EXTENSIONS: &[(&str, &str)] = &[
    ("md", "md"),
    ("txt", "txt"),
    ("html", "html"),
    ("docx", "docx"),
];

#[derive(Clone, Serialize)]
pub struct SaveResult {
    pub path: String,
    pub format: String,
    pub bytes: usize,
}

/// Remove qualquer noção de caminho e caracteres problemáticos do nome que
/// o modelo escolheu. Resultado: um "stem" seguro para join na pasta.
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next() // descarta qualquer componente de path
        .unwrap_or("");
    let mut out = String::new();
    for c in base.chars() {
        // inválidos no Windows + controle + nulo viram espaço (não some
        // colado: "que?é" → "que é")
        if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // colapsa espaços repetidos (inclusive os criados acima)
    let mut collapsed = String::new();
    let mut last_space = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !last_space && !collapsed.is_empty() {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    let out = collapsed.trim().trim_matches('.').trim();
    // limita o stem (chars, não bytes — UTF-8 válido por construção)
    let out: String = out.chars().take(100).collect();
    if out.is_empty() {
        "documento".to_string()
    } else {
        out
    }
}

/// Caminho final: pasta + stem + extensão, sem sobrescrever (sufixo -N).
/// Confirma que o resultado continua DENTRO da pasta (defesa em profundidade:
/// o stem sanitizado já não pode escapar, mas a checagem é barata).
fn resolve_path(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|e| format!("Pasta inválida: {e}"))?;
    let candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.starts_with(&dir) {
        return Err("Caminho fora da pasta de destino".into());
    }
    if !candidate.exists() {
        return Ok(candidate);
    }
    for n in 1..=99 {
        let c = dir.join(format!("{stem}-{n}.{ext}"));
        if !c.exists() {
            return Ok(c);
        }
    }
    Err("Já existem 100 arquivos com este nome na pasta".into())
}

/// Wrapper HTML offline mínimo (sem CDN, sem fontes externas) para o formato
/// "página html". O corpo é o markdown do modelo convertido a HTML simples.
fn html_shell(title: &str, body_html: &str) -> String {
    let title = escape_html(title);
    format!(
        "<!DOCTYPE html>\n<html lang=\"pt\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n<style>\n\
body {{ max-width: 760px; margin: 40px auto; padding: 0 20px; \
font-family: system-ui, -apple-system, 'Segoe UI', sans-serif; \
line-height: 1.6; color: #1a1a2e; background: #fafafa; }}\n\
h1,h2,h3 {{ line-height: 1.25; }}\n\
code,pre {{ background: #eee; border-radius: 6px; }}\n\
pre {{ padding: 12px; overflow-x: auto; }}\n\
table {{ border-collapse: collapse; }}\n\
td,th {{ border: 1px solid #ccc; padding: 6px 10px; }}\n\
blockquote {{ border-left: 4px solid #ccc; margin-left: 0; \
padding-left: 14px; color: #555; }}\n\
img {{ max-width: 100%; }}\n\
</style>\n</head>\n<body>\n{body_html}\n</body>\n</html>\n"
    )
}

/// Escape HTML (o conteúdo vem do modelo, mas a página abre num browser).
/// concat! monta as entities em compile time.
pub fn escape_html(s: &str) -> String {
    // & primeiro, senão as entities seguintes seriam re-escapadas
    s.replace('&', concat!("&", "amp;"))
        .replace('<', concat!("&", "lt;"))
        .replace('>', concat!("&", "gt;"))
        .replace('"', concat!("&", "quot;"))
}

/// Conversão markdown → HTML mínima e offline (o front usa marked; aqui no
/// backend o alvo é "bom o bastante" para páginas simples: títulos, listas,
/// negrito/itálico/código, links, parágrafos). Escapa ANTES de marcar.
pub fn md_to_html(md: &str) -> String {
    let esc = escape_html;
    let mut out = String::new();
    let mut in_list = false;
    let mut in_code = false;
    let mut in_para: Option<String> = None;

    let mut flush_para = |out: &mut String, in_para: &mut Option<String>| {
        if let Some(p) = in_para.take() {
            out.push_str(&format!("<p>{}</p>\n", inline(&esc(&p))));
        }
    };

    for line in md.lines() {
        let t = line.trim();
        if in_code {
            if t.starts_with("```") {
                out.push_str("</code></pre>\n");
                in_code = false;
            } else {
                out.push_str(&esc(line));
                out.push('\n');
            }
            continue;
        }
        if t.starts_with("```") {
            flush_para(&mut out, &mut in_para);
            if in_list {
                out.push_str("</ul>\n");
                in_list = false;
            }
            out.push_str("<pre><code>");
            in_code = true;
            continue;
        }
        // headings
        let heading = t
            .strip_prefix("###")
            .map(|s| (3, s))
            .or_else(|| t.strip_prefix("##").map(|s| (2, s)))
            .or_else(|| t.strip_prefix("#").map(|s| (1, s)));
        if let Some((lvl, rest)) = heading {
            let rest = rest.trim();
            if !rest.is_empty() && !rest.starts_with('#') {
                flush_para(&mut out, &mut in_para);
                if in_list {
                    out.push_str("</ul>\n");
                    in_list = false;
                }
                out.push_str(&format!("<h{lvl}>{}</h{lvl}>\n", inline(&esc(rest))));
                continue;
            }
        }
        // listas
        if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush_para(&mut out, &mut in_para);
            if !in_list {
                out.push_str("<ul>\n");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>\n", inline(&esc(item))));
            continue;
        }
        if t.is_empty() {
            flush_para(&mut out, &mut in_para);
            if in_list {
                out.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }
        match in_para.take() {
            Some(mut p) => {
                p.push(' ');
                p.push_str(t);
                in_para = Some(p);
            }
            None => in_para = Some(t.to_string()),
        }
    }
    flush_para(&mut out, &mut in_para);
    if in_list {
        out.push_str("</ul>\n");
    }
    if in_code {
        out.push_str("</code></pre>\n");
    }
    out
}

/// marcações inline: **negrito**, *itálico*, `código`, [texto](url)
fn inline(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("**") {
            if let Some(end) = rest[2..].find("**") {
                let inner = &rest[2..2 + end];
                if !inner.is_empty() {
                    out.push_str(&format!("<strong>{}</strong>", inline(inner)));
                    i += 2 + inner.chars().count() + 2;
                    continue;
                }
            }
        }
        if rest.starts_with('*') && !rest.starts_with("**") {
            if let Some(end) = rest[1..].find('*') {
                let inner = &rest[1..1 + end];
                if !inner.is_empty() {
                    out.push_str(&format!("<em>{}</em>", inline(inner)));
                    i += 1 + inner.chars().count() + 1;
                    continue;
                }
            }
        }
        if rest.starts_with('`') {
            if let Some(end) = rest[1..].find('`') {
                let inner = &rest[1..1 + end];
                out.push_str(&format!("<code>{}</code>", inner));
                i += 1 + inner.chars().count() + 1;
                continue;
            }
        }
        if rest.starts_with('[') {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') {
                    let text = &rest[1..close];
                    let url = &rest[close + 2..close + 2 + end];
                    // só http(s): o link é gerado pelo modelo
                    if url.starts_with("http://") || url.starts_with("https://") {
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            url,
                            inline(text)
                        ));
                        i += 1 + text.chars().count() + 2 + url.chars().count() + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown → .docx (Word de verdade, puro Rust — zero binário extra).
// Cobre o que modelos pequenos produzem de fato: títulos, parágrafos,
// negrito/itálico, listas (bullet/numeradas com aninhamento), blocos de
// código e réguas. Tabelas viram parágrafos (texto preservado, não perdido).
// O modelo só escreve MARKDOWN — a conversão é nossa (é o que torna a
// ferramenta confiável em modelos pequenos).
// ---------------------------------------------------------------------------

use docx_rs::{BreakType, Docx, Paragraph, Run};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

fn heading_number(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

pub fn markdown_to_docx(md: &str) -> Result<Vec<u8>, String> {
    let mut docx = Docx::new();
    let mut para: Option<Paragraph> = None;
    let mut bold = 0usize;
    let mut italic = 0usize;
    let mut heading: Option<u8> = None;
    let mut in_code = false;
    let mut code_buf = String::new();
    // pilha de listas: None = bullet; Some(n) = numerada a partir de n
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut counters: Vec<u64> = Vec::new();
    let mut link_url: Option<String> = None;

    let push = |para: &mut Option<Paragraph>, run: Run| {
        if let Some(p) = para.take() {
            *para = Some(p.add_run(run));
        }
    };
    let inline_run = |text: &str, bold: bool, italic: bool, heading: Option<u8>| {
        let mut r = Run::new().add_text(text);
        if bold || heading.is_some() {
            r = r.bold();
        }
        if italic {
            r = r.italic();
        }
        if let Some(l) = heading {
            // half-points: h1 18pt, h2 16pt, h3 14pt, h4+ 12pt
            let size = match l {
                1 => 36,
                2 => 32,
                3 => 28,
                _ => 24,
            };
            r = r.size(size);
        }
        r
    };

    for ev in Parser::new(md) {
        match ev {
            Event::Start(Tag::Paragraph) | Event::Start(Tag::BlockQuote(_)) => {
                para = Some(Paragraph::new());
            }
            Event::End(TagEnd::Paragraph) | Event::End(TagEnd::BlockQuote(_)) => {
                if let Some(p) = para.take() {
                    docx = docx.add_paragraph(p);
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let l = heading_number(level);
                heading = Some(l);
                para = Some(Paragraph::new().style(&format!("Heading{l}")));
            }
            Event::End(TagEnd::Heading(_)) => {
                heading = None;
                if let Some(p) = para.take() {
                    docx = docx.add_paragraph(p);
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::List(start)) => {
                lists.push(start);
                counters.push(start.unwrap_or(0));
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                counters.pop();
            }
            Event::Start(Tag::Item) => {
                // aninhamento visual por indentação no próprio texto (sem a
                // API de numbering: menos superfície, mesmo resultado no Word)
                let depth = lists.len().saturating_sub(1);
                let indent = "    ".repeat(depth);
                let prefix = match lists.last() {
                    Some(Some(_)) => {
                        let c = counters.last_mut().unwrap();
                        *c += 1;
                        format!("{indent}{}. ", c)
                    }
                    _ => format!("{indent}• "),
                };
                para = Some(Paragraph::new().add_run(Run::new().add_text(prefix)));
            }
            Event::End(TagEnd::Item) => {
                if let Some(p) = para.take() {
                    docx = docx.add_paragraph(p);
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                let text = code_buf.trim_end().to_string();
                code_buf.clear();
                for line in text.lines() {
                    docx = docx.add_paragraph(
                        Paragraph::new().add_run(Run::new().add_text(if line.is_empty() {
                            " "
                        } else {
                            line
                        })),
                    );
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_url = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = link_url.take() {
                    // só http(s) (o link vem do modelo)
                    if url.starts_with("http://") || url.starts_with("https://") {
                        push(&mut para, Run::new().add_text(format!(" ({url})")));
                    }
                }
            }
            Event::Text(t) => {
                if in_code {
                    code_buf.push_str(&t);
                } else {
                    if para.is_none() {
                        // texto solto (ex.: célula de tabela): não perder
                        para = Some(Paragraph::new());
                    }
                    push(
                        &mut para,
                        inline_run(&t, bold > 0, italic > 0, heading),
                    );
                }
            }
            Event::Code(t) => {
                if !in_code {
                    if para.is_none() {
                        para = Some(Paragraph::new());
                    }
                    push(&mut para, Run::new().add_text(t.to_string()));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code {
                    code_buf.push('\n');
                } else {
                    push(&mut para, Run::new().add_break(BreakType::TextWrapping));
                }
            }
            Event::Rule => {
                if let Some(p) = para.take() {
                    docx = docx.add_paragraph(p);
                }
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text("―――――")),
                );
            }
            Event::TaskListMarker(checked) => {
                push(
                    &mut para,
                    Run::new().add_text(if checked { "☑ " } else { "☐ " }),
                );
            }
            _ => {}
        }
    }
    // parágrafo pendente (markdown truncado, caso raro)
    if let Some(p) = para.take() {
        docx = docx.add_paragraph(p);
    }

    let mut buf = std::io::Cursor::new(Vec::new());
    docx.pack(&mut buf).map_err(|e| format!("docx: {e}"))?;
    Ok(buf.into_inner())
}

/// Salva o documento na pasta escolhida pelo usuário. `content` é markdown
/// (o que LLMs geram de melhor); a conversão para o formato pedido é NOSSA.
pub fn save_document(
    dir: &str,
    filename: &str,
    format: &str,
    content: &str,
) -> Result<SaveResult, String> {
    let format = format.to_lowercase();
    let ext = EXTENSIONS
        .iter()
        .find(|(f, _)| *f == format)
        .map(|(_, e)| *e)
        .ok_or_else(|| format!("Formato não suportado: {format}"))?;
    if content.len() > MAX_CONTENT_BYTES {
        return Err("Conteúdo maior que 2 MB".into());
    }
    let stem = sanitize_filename(filename);
    let path = resolve_path(Path::new(dir), &stem, ext)?;
    let bytes = match format.as_str() {
        "html" => html_shell(&stem, &md_to_html(content)).into_bytes(),
        "docx" => markdown_to_docx(content)?,
        _ => content.as_bytes().to_vec(),
    };
    std::fs::write(&path, &bytes).map_err(|e| format!("Gravando {path:?}: {e}"))?;
    Ok(SaveResult {
        path: path.to_string_lossy().into_owned(),
        format,
        bytes: bytes.len(),
    })
}

/// Abre o explorer/gerenciador mostrando o arquivo (botão "abrir pasta").
pub fn open_in_folder(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err("Arquivo não existe".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &p.display().to_string()])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let dir = p.parent().unwrap_or(Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Pasta padrão para salvar documentos: ~/Documents (cria se não existir).
pub fn default_save_dir() -> String {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join("Documents");
    let dir = if dir.exists() { dir } else { home };
    dir.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_descarta_path_e_invalidos() {
        assert_eq!(sanitize_filename("relatorio"), "relatorio");
        assert_eq!(
            sanitize_filename("..\\..\\windows\\system32\\evil"),
            "evil" // só o último componente
        );
        assert_eq!(sanitize_filename("a/b/c.txt"), "c.txt");
        assert_eq!(sanitize_filename("o que?<é>:isso*"), "o que é isso");
        assert_eq!(sanitize_filename("   ...   "), "documento");
        assert_eq!(sanitize_filename(""), "documento");
        let long = "x".repeat(300);
        assert_eq!(sanitize_filename(&long).chars().count(), 100);
    }

    #[test]
    fn resolve_nao_sobrescreve() {
        let tmp = std::env::temp_dir().join(format!("localai-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let a = resolve_path(&tmp, "doc", "md").unwrap();
        std::fs::write(&a, "x").unwrap();
        let b = resolve_path(&tmp, "doc", "md").unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().contains("doc-1"));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn save_rejeita_formato_desconhecido() {
        let tmp = std::env::temp_dir().join(format!("localai-test2-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(save_document(&tmp.to_string_lossy(), "d", "exe", "x").is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn save_html_gera_documento_valido() {
        let tmp = std::env::temp_dir().join(format!("localai-test3-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir = tmp.to_string_lossy().into_owned();
        let res = save_document(&dir, "página", "html", "# Título\n\n- a\n- b\n")
            .unwrap();
        let html = std::fs::read_to_string(&res.path).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<h1>Título</h1>"));
        assert!(html.contains("<li>a</li>"));
        assert!(html.contains("<li>b</li>"));
        assert!(!html.contains("http://cdn")); // offline
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn md_to_html_escapa_e_marca() {
        let out = md_to_html("texto **forte** e `code` e <script>alert(1)</script>");
        assert!(out.contains("<strong>forte</strong>"));
        assert!(out.contains("<code>code</code>"));
        // o tag vira entity: nada de HTML executável vindo do modelo
        let lt = format!("{}lt;", '&');
        let gt = format!("{}gt;", '&');
        assert!(out.contains(&format!("{lt}script{gt}")));
        assert!(!out.contains("<script"));
    }

    #[test]
    fn md_to_html_link_somente_http() {
        let ok = md_to_html("[site](https://exemplo.com)");
        assert!(ok.contains("<a href=\"https://exemplo.com\">site</a>"));
        // javascript: é recusado (vira texto)
        let bad = md_to_html("[x](javascript:alert(1))");
        assert!(!bad.contains("href=\"javascript"));
    }

    #[test]
    fn md_to_docx_gera_zip_valido() {
        let bytes = markdown_to_docx(
            "# Título\n\nUm parágrafo **forte** e *calmo*.\n\n- um\n- dois\n\n## Seção\n\n```\ncodigo()\n```\n",
        )
        .unwrap();
        // assinatura ZIP (PK\x03\x04) — .docx é um zip de XML
        assert!(bytes.len() > 500, "docx muito pequeno: {} bytes", bytes.len());
        assert_eq!(&bytes[0..2], b"PK");
    }

    #[test]
    fn save_document_docx_grava_arquivo() {
        let tmp = std::env::temp_dir().join(format!("localai-test4-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir = tmp.to_string_lossy().into_owned();
        let res = save_document(&dir, "meu-doc", "docx", "# Oi\n\ntexto").unwrap();
        assert!(res.path.ends_with(".docx"));
        let bytes = std::fs::read(&res.path).unwrap();
        assert_eq!(&bytes[0..2], b"PK");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn markdown_nao_perde_texto_de_tabela() {
        // Tabela não vira tabela no docx (decisão: simplicidade), mas o
        // texto das células NÃO pode sumir.
        let bytes = markdown_to_docx("| a | b |\n|---|---|\n| 1 | 2 |\n").unwrap();
        assert!(bytes.len() > 500);
    }
}
