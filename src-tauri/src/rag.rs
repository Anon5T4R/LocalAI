// RAG local: ingestão de PDF (texto por página → chunks) e busca BM25
// (SQLite FTS5). Tudo offline, no app_data do usuário, escopado por conversa.
//
// Desenho (combinado com o João, 2026-10-21):
// - PDF pequeno que cabe no contexto → o TEXTO INTEIRO vai pro prompt, sem
//   retrieval (a decisão é no front, `decideStrategy` em src/rag.ts).
// - PDF grande → busca BM25 top-k; os trechos entram ANEXADOS à pergunta e
//   visíveis no painel "Fontes" da UI. NENHUMA instrução oculta é injetada —
//   o app é só o bibliotecário: busca e entrega conteúdo.
// - OCR de scans (fatia D) e embeddings semânticos (fatia E) vêm depois;
//   `ocr_pages` já existe no resultado pra UI informar.
//
// Invariantes do chunking (provadas nos testes):
//   1. todo chunk ≤ MAX_CHARS (o overlap nunca empurra além do teto porque
//      pedaços de parágrafo gigante são pré-fatios em ≤ MAX-OVERLAP);
//   2. chunks vizinhos compartilham a cauda (overlap) — frases na fronteira
//      não se perdem;
//   3. nenhuma quebra no meio de um caractere UTF-8.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Alvo de tamanho por chunk (~500 tokens); teto duro e overlap de cauda
/// entre chunks vizinhos (~75 tokens). Chars ≈ 4×tokens em PT/EN/ES.
const TARGET_CHARS: usize = 2000;
const MAX_CHARS: usize = 3000;
const OVERLAP_CHARS: usize = 300;
/// Pedaços de parágrafos gigantes são pré-fatios neste limite para o
/// overlap caber no teto: OVERLAP + '\n' separador + pedaço ≤ MAX_CHARS.
const SPLIT_LIMIT: usize = MAX_CHARS - OVERLAP_CHARS - 1;
/// PDF maior que isto é recusado (a extração carrega o doc em RAM).
const MAX_DOC_BYTES: u64 = 100 * 1024 * 1024;
/// Teto do XML de um .docx (defesa contra zip bomb: o document.xml é
/// normalmente < 5 MB; 64 MB dá folga e limita o estrago).
const MAX_DOCXML_BYTES: u64 = 64 * 1024 * 1024;
/// Menos texto que isto no documento inteiro = provável scan sem camada de
/// texto (OCR vem na fatia D; por ora é erro explícito, não silêncio).
const MIN_DOC_CHARS: usize = 30;
/// Página com menos texto que isto é tratada como scan → tenta OCR.
const MIN_PAGE_CHARS: usize = 30;

#[derive(Clone, Serialize)]
pub struct DocMeta {
    pub doc_id: String,
    pub name: String,
    pub pages: u32,
    pub chars: u64,
}

#[derive(Clone, Serialize)]
pub struct AddDocResult {
    pub meta: DocMeta,
    /// Páginas recuperadas por OCR (sempre 0 até a fatia D).
    pub ocr_pages: u32,
}

#[derive(Clone, Serialize)]
pub struct Chunk {
    pub doc_id: String,
    pub doc_name: String,
    pub page: u32,
    pub seq: u32,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Banco (SQLite FTS5 no app_data; uma conexão por chamada — barato e sem
// estado global). Tokenizer unicode61 + remove_diacritics: "rescisão" e
// "rescisao" são o mesmo termo (essencial em PT/ES).
// ---------------------------------------------------------------------------

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Sem diretorio de dados do app: {e}"))?;
    Ok(dir.join("rag.db"))
}

pub fn open_db(app: &AppHandle) -> Result<Connection, String> {
    let path = db_path(app)?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("Nao criei {p:?}: {e}"))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("rag.db: {e}"))?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks USING fts5(\
         text, conv_id UNINDEXED, doc_id UNINDEXED, doc_name UNINDEXED, \
         page UNINDEXED, seq UNINDEXED, \
         tokenize='unicode61 remove_diacritics 2');\
         CREATE TABLE IF NOT EXISTS rag_vecs (\
         conv_id TEXT NOT NULL, doc_id TEXT NOT NULL, seq INTEGER NOT NULL, \
         dim INTEGER NOT NULL, vec BLOB NOT NULL, \
         PRIMARY KEY (conv_id, doc_id, seq));",
    )
    .map_err(|e| e.to_string())
}

/// Conexão em memória com o mesmo schema (testes).
#[cfg(test)]
pub(crate) fn mem_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

fn paragraphs(text: &str) -> Vec<String> {
    let norm = text.replace("\r\n", "\n");
    let mut out = Vec::new();
    for p in norm.split("\n\n") {
        let t = p.trim_matches('\n').trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    out
}

/// Fatiar um parágrafo gigante em pedaços ≤ SPLIT_LIMIT, cortando em quebra
/// de linha quando possível e nunca no meio de um caractere UTF-8.
fn split_big(para: &str) -> Vec<String> {
    if para.len() <= SPLIT_LIMIT {
        return vec![para.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = para;
    while !rest.is_empty() {
        if rest.len() <= SPLIT_LIMIT {
            out.push(rest.to_string());
            break;
        }
        // corte seco seguro: recua até uma fronteira UTF-8 válida ANTES de
        // fatiar o window (chars UTF-8 têm ≤ 4 bytes, recua no máximo 3)
        let mut hard = SPLIT_LIMIT;
        while !rest.is_char_boundary(hard) {
            hard -= 1;
        }
        let window = &rest[..hard];
        let cut = match window.rfind('\n') {
            // corta ANTES da quebra: o head termina em linha completa
            Some(i) if i > 0 => i,
            _ => hard,
        };
        let (head, tail) = rest.split_at(cut);
        let head = head.trim_matches('\n');
        if !head.is_empty() {
            out.push(head.to_string());
        }
        rest = tail.trim_start_matches('\n');
    }
    out
}

/// Cauda do buffer (últimos ~OVERLAP_CHARS, alinhada ao início da linha
/// seguinte) que vira o prefixo do próximo chunk. Buffers curtos não geram
/// overlap — senão o chunk seguinte repetiria quase o anterior inteiro.
fn overlap_tail(buf: &str) -> String {
    if buf.len() <= OVERLAP_CHARS * 2 {
        return String::new();
    }
    let start = {
        let s = buf.len() - OVERLAP_CHARS;
        let mut s = s;
        while !buf.is_char_boundary(s) {
            s += 1;
        }
        s
    };
    let tail = &buf[start..];
    match tail.find('\n') {
        Some(idx) if idx + 1 < tail.len() => tail[idx + 1..].to_string(),
        _ => tail.to_string(),
    }
}

fn flush(buf: &mut String, chunks: &mut Vec<String>) {
    let trimmed = buf.trim().to_string();
    if !trimmed.is_empty() {
        chunks.push(trimmed);
    }
    let tail = overlap_tail(buf);
    buf.clear();
    buf.push_str(&tail);
}

/// Fatiar o texto de UMA página em chunks ~TARGET_CHARS com overlap de cauda.
pub fn chunk_page(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut buf = String::new();
    for para in paragraphs(text) {
        for piece in split_big(&para) {
            if !buf.trim().is_empty() && buf.len() + piece.len() > TARGET_CHARS {
                flush(&mut buf, &mut chunks);
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&piece);
        }
    }
    // último chunk: sem overlap (não há vizinho à frente)
    let trimmed = buf.trim().to_string();
    if !trimmed.is_empty() {
        chunks.push(trimmed);
    }
    chunks
}

// ---------------------------------------------------------------------------
// Query FTS5: cada palavra entre aspas (escapada), unidas por OR — recall
// alto e o BM25 do FTS5 faz o ranking. Pontuação é descartada; palavras de
// 1 char são ruído. Sem termos → None (busca vazia, não erro).
// ---------------------------------------------------------------------------

/// Stop words ultra-frequentes de PT/EN/ES: em corpus pequeno (um PDF) elas
/// aparecem em quase todo chunk e viram ruído no ranking do BM25. Negativos
/// (não/not/no) ficam FORA da lista — carregam semântica na pergunta.
const STOPWORDS: &[&str] = &[
    // PT
    "as", "os", "de", "da", "do", "das", "dos", "é", "em", "no", "na", "nos",
    "nas", "um", "uma", "uns", "umas", "para", "pra", "com", "por", "que",
    "se", "ao", "aos", "às", "mais", "como", "mas", "seu", "sua", "meu",
    "minha",
    // EN
    "an", "the", "of", "to", "in", "on", "at", "is", "are", "was", "were",
    "be", "been", "and", "or", "for", "with", "that", "this", "it", "as",
    "by",
    // ES
    "el", "la", "los", "las", "del", "y", "un", "una", "unos", "unas",
    "con", "al", "su", "sus",
];

pub fn fts_query(query: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in query.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric());
        if w.chars().count() < 2 {
            continue;
        }
        if STOPWORDS.contains(&w.to_lowercase().as_str()) {
            continue;
        }
        let escaped = format!("\"{}\"", w.replace('"', "\"\""));
        if !terms.contains(&escaped) {
            terms.push(escaped);
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

// ---------------------------------------------------------------------------
// Operações
// ---------------------------------------------------------------------------

fn insert_chunk(
    conn: &Connection,
    conv_id: &str,
    doc_id: &str,
    doc_name: &str,
    page: u32,
    seq: u32,
    text: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO rag_chunks(text, conv_id, doc_id, doc_name, page, seq) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![text, conv_id, doc_id, doc_name, page, seq],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn new_doc_id(path: &std::path::Path) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write(path.to_string_lossy().as_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    h.write(&nanos.to_le_bytes());
    format!("d{:016x}", h.finish())
}

/// Caiu a extração do PDF: separa as páginas. Os outros formatos entram
/// como texto contínuo (page 0 = sem paginação).
fn doc_kind(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => Some("pdf"),
        Some("docx") => Some("docx"),
        Some("txt") => Some("txt"),
        Some("md") => Some("md"),
        _ => None,
    }
}

/// Desfaz as entidades XML básicas (o texto do Word escapa &, <, >, ").
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // &amp; por último (senão re-escaparia as anteriores)
        .replace("&amp;", "&")
}

/// Extrai o texto do WordprocessingML do .docx: junta o conteúdo de
/// `<w:t>`, transforma `</w:p>` em quebra de linha, `<w:br>`/`<w:cr>` em
/// newline e `<w:tab/>` em tab. Parser de tags simples de propósito: o
/// formato é estável e assim não dependemos da API de um parser XML.
pub fn docx_xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if tag == "/w:p" {
            out.push('\n');
        } else if tag.starts_with("w:br") || tag.starts_with("w:cr") {
            out.push('\n');
        } else if tag.starts_with("w:tab") {
            out.push('\t');
        } else if tag == "w:t" || tag.starts_with("w:t ") {
            // conteúdo até o fechamento; o texto vem escapado, sem tags XML
            if let Some(end) = rest.find("</w:t>") {
                out.push_str(&decode_entities(&rest[..end]));
                rest = &rest[end + "</w:t>".len()..];
            }
        }
    }
    // colapsa linhas vazias repetidas (o Word emite muitos parágrafos vazios)
    let mut lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            if lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                continue;
            }
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// Texto de um .docx (abre o zip e lê `word/document.xml`).
fn extract_docx_text(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("docx: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("docx: {e}"))?;
    let entry = zip
        .by_name("word/document.xml")
        .map_err(|_| "docx sem word/document.xml (arquivo inválido?)".to_string())?;
    let mut xml = String::new();
    entry
        .take(MAX_DOCXML_BYTES)
        .read_to_string(&mut xml)
        .map_err(|e| format!("docx: {e}"))?;
    Ok(docx_xml_to_text(&xml))
}

/// Extrai o documento por "página": (textos, é paginado?).
/// PDF é paginado (página real); docx/txt/md são texto contínuo (page 0).
fn extract_document(path: &std::path::Path) -> Result<(Vec<String>, bool), String> {
    match doc_kind(path) {
        Some("pdf") => {
            let pages =
                pdf_extract::extract_text_by_pages(path).map_err(|e| format!("PDF: {e}"))?;
            Ok((pages, true))
        }
        Some("docx") => Ok((vec![extract_docx_text(path)?], false)),
        Some(_) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("texto: {e}"))?;
            Ok((vec![text], false))
        }
        None => Err("UNSUPPORTED".into()),
    }
}

/// Extrai o texto do PDF por página, fatia em chunks e indexa na conversa.
/// Erros com código estável no prefixo (o front mapeia pra i18n):
/// "TOO_BIG", "NO_TEXT" (scan sem camada de texto e sem OCR) e "UNSUPPORTED".
pub fn ingest_pdf(app: &AppHandle, path: &str, conv_id: &str) -> Result<AddDocResult, String> {
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err(format!("Arquivo nao encontrado: {path}"));
    }
    let meta = std::fs::metadata(&p).map_err(|e| e.to_string())?;
    if meta.len() > MAX_DOC_BYTES {
        return Err("TOO_BIG".into());
    }
    let (pages, paginated) = extract_document(&p)?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "documento".into());
    // Página de PDF sem camada de texto = provável scan → OCR (Windows
    // nativo). docx/txt/md são texto contínuo: não passam por OCR.
    let mut ocr_pages = 0u32;
    let mut page_texts: Vec<String> = Vec::with_capacity(pages.len());
    for (i, page_text) in pages.iter().enumerate() {
        let mut text = page_text.clone();
        if paginated && text.trim().len() < MIN_PAGE_CHARS {
            if let Some(ocr_text) = crate::ocr::page_text(&p, (i + 1) as u32) {
                text = ocr_text;
                ocr_pages += 1;
            }
        }
        page_texts.push(text);
    }
    let total_chars: usize = page_texts.iter().map(|s| s.trim().len()).sum();
    if total_chars < MIN_DOC_CHARS {
        return Err("NO_TEXT".into());
    }
    let doc_id = new_doc_id(&p);
    let conn = open_db(app)?;
    // re-ingest do mesmo id não duplica (defensivo; id é novo a cada anexo)
    remove_doc(&conn, conv_id, &doc_id)?;
    let mut seq = 0u32;
    for (i, page_text) in page_texts.iter().enumerate() {
        // 0 = documento sem paginação (docx/txt/md) — a citação sai só com o nome
        let page = if paginated { (i + 1) as u32 } else { 0 };
        for chunk in chunk_page(page_text) {
            insert_chunk(&conn, conv_id, &doc_id, &name, page, seq, &chunk)?;
            seq += 1;
        }
    }
    Ok(AddDocResult {
        meta: DocMeta {
            doc_id,
            name,
            pages: if paginated { page_texts.len() as u32 } else { 0 },
            chars: total_chars as u64,
        },
        ocr_pages,
    })
}

pub fn search(conn: &Connection, conv_id: &str, query: &str, k: u32) -> Result<Vec<Chunk>, String> {
    let Some(match_expr) = fts_query(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT text, doc_id, doc_name, page, seq FROM rag_chunks \
             WHERE rag_chunks MATCH ?1 AND conv_id = ?2 \
             ORDER BY rank LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![match_expr, conv_id, k as i64], |r| {
            Ok(Chunk {
                text: r.get(0)?,
                doc_id: r.get(1)?,
                doc_name: r.get(2)?,
                page: r.get(3)?,
                seq: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Texto completo de um documento (modo "cabe no contexto"), com marcador
/// de página entre páginas para o modelo poder citar "p. N".
pub fn doc_text(conn: &Connection, conv_id: &str, doc_id: &str) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT page, text FROM rag_chunks \
             WHERE conv_id = ?1 AND doc_id = ?2 ORDER BY page, seq",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![conv_id, doc_id], |r| {
            Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    let mut any = false;
    let mut last_page = 0u32;
    for row in rows {
        let (page, text) = row.map_err(|e| e.to_string())?;
        if !any {
            // page 0 = documento sem paginação (docx/txt/md): sem marcador
            if page > 0 {
                out.push_str(&format!("[p. {page}]\n"));
            }
        } else if page != last_page && page > 0 {
            out.push_str(&format!("\n\n[p. {page}]\n"));
        } else {
            out.push_str("\n\n");
        }
        out.push_str(&text);
        last_page = page;
        any = true;
    }
    Ok(if any { Some(out) } else { None })
}

pub fn remove_doc(conn: &Connection, conv_id: &str, doc_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM rag_chunks WHERE conv_id = ?1 AND doc_id = ?2",
        params![conv_id, doc_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Limpa tudo de uma conversa (chamado ao apagar a conversa na UI).
pub fn remove_conv(conn: &Connection, conv_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM rag_chunks WHERE conv_id = ?1", params![conv_id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM rag_vecs WHERE conv_id = ?1", params![conv_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Busca semântica (fatia E): vetores no mesmo rag.db + fusão RRF com o BM25.
// ---------------------------------------------------------------------------

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Chunks ainda SEM vetor (precisam ser embedados).
pub fn pending_chunks(
    conn: &Connection,
    conv_id: &str,
) -> Result<Vec<(String, u32, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.doc_id, c.seq, c.text FROM rag_chunks c \
             LEFT JOIN rag_vecs v ON v.conv_id = c.conv_id \
               AND v.doc_id = c.doc_id AND v.seq = c.seq \
             WHERE c.conv_id = ?1 AND v.seq IS NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![conv_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn add_vec(
    conn: &Connection,
    conv_id: &str,
    doc_id: &str,
    seq: u32,
    vec: &[f32],
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO rag_vecs(conv_id, doc_id, seq, dim, vec) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![conv_id, doc_id, seq, vec.len() as i64, vec_to_blob(vec)],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Todos os vetores de uma conversa: (doc_id, seq, vetor).
pub fn all_vecs(
    conn: &Connection,
    conv_id: &str,
) -> Result<Vec<(String, u32, Vec<f32>)>, String> {
    let mut stmt = conn
        .prepare("SELECT doc_id, seq, vec FROM rag_vecs WHERE conv_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![conv_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                blob_to_vec(&r.get::<_, Vec<u8>>(2)?),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// O chunk correspondente a (doc_id, seq) — texto/página para a citação.
pub fn chunk_by_key(
    conn: &Connection,
    conv_id: &str,
    doc_id: &str,
    seq: u32,
) -> Result<Option<Chunk>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT text, doc_name, page FROM rag_chunks \
             WHERE conv_id = ?1 AND doc_id = ?2 AND seq = ?3 LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![conv_id, doc_id, seq], |r| {
            Ok(Chunk {
                text: r.get(0)?,
                doc_id: doc_id.to_string(),
                doc_name: r.get(1)?,
                page: r.get(2)?,
                seq,
            })
        })
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(row) => Ok(Some(row.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Similaridade de cosseno (vetores normalizados → dot; mas normaliza por
/// segurança: nem todo modelo sai normalizado).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Reciprocal Rank Fusion: funde os rankings léxico (BM25) e semântico sem
/// comparar escalas de score (o padrão prático da indústria para híbrido).
pub fn rrf_fuse(bm25: &[Chunk], semantic: &[Chunk], k: usize) -> Vec<Chunk> {
    const RRF_K: f64 = 60.0;
    use std::collections::HashMap;
    let mut acc: HashMap<(String, u32), (f64, Chunk)> = HashMap::new();
    let mut push = |list: &[Chunk], acc: &mut HashMap<(String, u32), (f64, Chunk)>| {
        for (i, c) in list.iter().enumerate() {
            let s = 1.0 / (RRF_K + (i as f64 + 1.0));
            let e = acc
                .entry((c.doc_id.clone(), c.seq))
                .or_insert_with(|| (0.0, c.clone()));
            e.0 += s;
        }
    };
    push(bm25, &mut acc);
    push(semantic, &mut acc);
    let mut out: Vec<(f64, Chunk)> = acc.into_values().collect();
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().take(k).map(|(_, c)| c).collect()
}

// ---------------------------------------------------------------------------
// Testes (padrão da suíte: lógica determinística ganha #[test])
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_curto_vira_um_so_chunk() {
        let out = chunk_page("oi mundo\n\nsegundo parágrafo");
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("oi mundo"));
        assert!(out[0].contains("segundo parágrafo"));
    }

    #[test]
    fn chunk_respeita_teto_duro() {
        // parágrafo gigante (4800 chars) repetido: pedaços pré-fatios ≤ 2700,
        // e o overlap nunca pode empurrar um chunk além de MAX_CHARS
        let para: String = "palavra ".repeat(600);
        let text = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk_page(&text);
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(c.len() <= MAX_CHARS, "chunk de {} chars", c.len());
        }
    }

    #[test]
    fn chunk_tem_overlap_entre_vizinhos() {
        // 10 parágrafos de ~600 chars → vários chunks; o prefixo do chunk
        // seguinte deve ser um sufixo do anterior (a cauda compartilhada)
        let text = (0..10)
            .map(|i| format!("parágrafo número {i} ") + &"texto ".repeat(120))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk_page(&text);
        assert!(
            chunks.len() >= 2,
            "esperava vários chunks, veio {}",
            chunks.len()
        );
        let a = chunks[0].as_str();
        let b = chunks[1].as_str();
        let mut best = 0usize;
        for l in (50..=OVERLAP_CHARS).rev() {
            if l > a.len() || l > b.len() || !a.is_char_boundary(a.len() - l) {
                continue;
            }
            if b.starts_with(&a[a.len() - l..]) {
                best = l;
                break;
            }
        }
        assert!(best >= 50, "sem overlap detectável entre vizinhos (best={best})");
    }

    #[test]
    fn chunk_nao_quebra_utf8() {
        // multibyte no boundary potencial do corte seco
        let text = "ção ".repeat(2000);
        for c in chunk_page(&text) {
            assert!(std::str::from_utf8(c.as_bytes()).is_ok());
        }
    }

    #[test]
    fn fts_query_escapa_e_descarta_ruido() {
        // "como"/"de" são stop words (ruído no BM25 de corpus pequeno);
        // "o"/"a" caem no filtro de 1 char; "—" não tem alfanumérico
        let q = fts_query("Como rescindir \"o\" contrato? a — de");
        assert_eq!(q.as_deref(), Some("\"rescindir\" OR \"contrato\""));
        assert!(fts_query("a — ?!").is_none());
        // aspas internas são dobradas (escape do FTS5)
        assert_eq!(
            fts_query("termo\"com\"aspas"),
            Some("\"termo\"\"com\"\"aspas\"".to_string())
        );
    }

    #[test]
    fn busca_acha_termo_com_e_sem_acento() {
        let conn = mem_db();
        insert_chunk(
            &conn, "c1", "d1", "doc.pdf", 1, 0,
            "A cláusula de rescisão do contrato de aluguel prevê aviso de 30 dias.",
        )
        .unwrap();
        for q in ["rescisão", "rescisao", "aluguel contrato"] {
            let hits = search(&conn, "c1", q, 5).unwrap();
            assert_eq!(hits.len(), 1, "query {q:?} deveria achar 1 chunk");
        }
        // escopo por conversa: outra conversa não vê o chunk
        assert!(search(&conn, "c2", "rescisão", 5).unwrap().is_empty());
    }

    #[test]
    fn busca_respeita_k() {
        let conn = mem_db();
        for i in 0..6 {
            insert_chunk(
                &conn, "c1", "d1", "doc.pdf", 1, i,
                &format!("contrato contrato contrato número {i}"),
            )
            .unwrap();
        }
        let hits = search(&conn, "c1", "contrato", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn doc_text_marca_paginas() {
        let conn = mem_db();
        insert_chunk(&conn, "c1", "d1", "a.pdf", 1, 0, "página um").unwrap();
        insert_chunk(&conn, "c1", "d1", "a.pdf", 2, 1, "página dois").unwrap();
        let txt = doc_text(&conn, "c1", "d1").unwrap().unwrap();
        assert!(txt.contains("[p. 1]\npágina um"));
        assert!(txt.contains("[p. 2]\npágina dois"));
        assert!(doc_text(&conn, "c1", "inexistente").unwrap().is_none());
    }

    #[test]
    fn docx_xml_extrai_paragrafos_quebras_e_entidades() {
        let xml = "<w:body><w:p><w:r><w:t>Olá &amp; bem-vindo</w:t></w:r></w:p>\
<w:p><w:r><w:t xml:space=\"preserve\">Segundo</w:t><w:tab/><w:t>Terceiro</w:t></w:r></w:p>\
<w:p><w:r><w:t>quarta</w:t><w:br/><w:t>quinta</w:t></w:r></w:p></w:body>";
        let txt = docx_xml_to_text(xml);
        assert_eq!(txt, "Olá & bem-vindo\nSegundo\tTerceiro\nquarta\nquinta");
    }

    #[test]
    fn docx_roundtrip_com_o_gerador_do_app() {
        // pipeline completo de verdade: o MESMO gerador de .docx das
        // ferramentas escreve, e o extrator do anexo lê de volta
        let md = "# Título\n\nUm parágrafo com **forte** e *itálico*.\n\n- item um\n- item dois\n";
        let bytes = crate::tools::markdown_to_docx(md).unwrap();
        let tmp = std::env::temp_dir().join(format!("localai-rag-docx-{}.docx", std::process::id()));
        std::fs::write(&tmp, &bytes).unwrap();
        let text = extract_docx_text(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();
        assert!(text.contains("Título"), "texto: {text:?}");
        assert!(text.contains("forte"));
        assert!(text.contains("item um"));
        assert!(text.contains("item dois"));
    }

    #[test]
    fn extract_document_txt_e_md() {
        let tmp = std::env::temp_dir().join(format!("localai-rag-txt-{}.txt", std::process::id()));
        std::fs::write(&tmp, "conteudo simples").unwrap();
        let (pages, paginated) = extract_document(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();
        assert_eq!(pages, vec!["conteudo simples".to_string()]);
        assert!(!paginated);
        // extensão desconhecida → UNSUPPORTED (o front mostra erro claro)
        assert!(extract_document(std::path::Path::new("x.exe")).is_err());
    }

    #[test]
    fn remove_limpa_so_o_alvo() {
        let conn = mem_db();
        insert_chunk(&conn, "c1", "d1", "a.pdf", 1, 0, "alpha").unwrap();
        insert_chunk(&conn, "c1", "d2", "b.pdf", 1, 0, "beta").unwrap();
        remove_doc(&conn, "c1", "d1").unwrap();
        assert!(search(&conn, "c1", "alpha", 5).unwrap().is_empty());
        assert_eq!(search(&conn, "c1", "beta", 5).unwrap().len(), 1);
        remove_conv(&conn, "c1").unwrap();
        assert!(search(&conn, "c1", "beta", 5).unwrap().is_empty());
    }

    #[test]
    fn cosine_casos_basicos() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        // escalas diferentes, mesma direção → 1
        assert!((cosine(&[2.0, 2.0], &[1.0, 1.0]) - 1.0).abs() < 1e-6);
        // dimensões diferentes / vazio / zero → 0 (nunca NaN)
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn rrf_favorece_item_presente_nas_duas_listas() {
        let mk = |doc: &str, seq: u32| Chunk {
            doc_id: doc.into(),
            doc_name: "d.pdf".into(),
            page: 1,
            seq,
            text: format!("{doc}-{seq}"),
        };
        // bm25: A, B ; semantic: B, C  → B aparece nos dois e deve liderar
        let bm25 = vec![mk("A", 0), mk("B", 1)];
        let sem = vec![mk("B", 1), mk("C", 2)];
        let out = rrf_fuse(&bm25, &sem, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].doc_id, "B");
        // sem duplicata do item que aparece nas duas listas
        assert_eq!(out.iter().filter(|c| c.doc_id == "B").count(), 1);
    }

    #[test]
    fn vetores_roundtrip_e_pendentes() {
        let conn = mem_db();
        insert_chunk(&conn, "c1", "d1", "a.pdf", 1, 0, "um").unwrap();
        insert_chunk(&conn, "c1", "d1", "a.pdf", 1, 1, "dois").unwrap();
        let pending = pending_chunks(&conn, "c1").unwrap();
        assert_eq!(pending.len(), 2);

        add_vec(&conn, "c1", "d1", 0, &[0.5, -0.25, 0.125]).unwrap();
        let pending = pending_chunks(&conn, "c1").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1, 1); // o seq que falta

        let all = all_vecs(&conn, "c1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].2, vec![0.5, -0.25, 0.125]);

        // o chunk do vetor é recuperável para citação
        let c = chunk_by_key(&conn, "c1", "d1", 0).unwrap().unwrap();
        assert_eq!(c.text, "um");

        // remover a conversa limpa vetores também
        remove_conv(&conn, "c1").unwrap();
        assert!(all_vecs(&conn, "c1").unwrap().is_empty());
    }
}
