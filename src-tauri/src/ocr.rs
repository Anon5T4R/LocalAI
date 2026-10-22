// OCR mínimo para PDFs escaneados usando a API NATIVA do Windows
// (Windows.Media.Ocr) — o mesmo motor do Windows Lens. Zero download, zero
// binário extra no instalador, offline.
//
// Pipeline: a página de um scan é uma IMAGEM embutida no PDF (quase sempre
// JPEG/DCTDecode). Extraímos os bytes com lopdf, decodificamos via
// BitmapDecoder e reconhecemos com OcrEngine. PDFs com camada de texto nem
// passam por aqui — o extrator normal (pdf-extract) já resolve.
//
// Limitações honestas: só Windows (Linux/macOS ficam sem OCR → páginas de
// scan seguem vazias); exige pacote de idioma de OCR instalado (o Windows
// PT-BR já traz); scans de fax antigos (CCITT/JBIG2) não são extraídos —
// JPEG é o caso real de scanner moderno.

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};

    /// Bytes das imagens JPEG embutidas nesta página (filtro DCTDecode).
    /// JPX (JPEG 2000) e CCITT/JBIG2 ficam de fora (raro em scanner moderno).
    fn page_jpegs(path: &Path, page: u32) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let Ok(doc) = lopdf::Document::load(path) else {
            return out;
        };
        let Some(page_id) = doc.get_pages().get(&page).copied() else {
            return out;
        };
        let Ok(page_obj) = doc.get_object(page_id) else {
            return out;
        };
        let Ok(page_dict) = page_obj.as_dict() else {
            return out;
        };
        // Resources pode ser dict direto ou referência (ambos são comuns)
        let Some(res_obj) = page_dict.get(b"Resources").ok() else {
            return out;
        };
        let Ok((_, res)) = doc.dereference(res_obj) else {
            return out;
        };
        let Ok(res_dict) = res.as_dict() else {
            return out;
        };
        let Some(xobj_obj) = res_dict.get(b"XObject").ok() else {
            return out;
        };
        let Ok((_, xobjs)) = doc.dereference(xobj_obj) else {
            return out;
        };
        let Ok(xobj_dict) = xobjs.as_dict() else {
            return out;
        };
        for (_, val) in xobj_dict.iter() {
            let Ok((_, obj)) = doc.dereference(val) else {
                continue;
            };
            let Ok(stream) = obj.as_stream() else {
                continue;
            };
            let is_image = matches!(
                stream.dict.get(b"Subtype").ok(),
                Some(lopdf::Object::Name(n)) if n == b"Image"
            );
            if !is_image {
                continue;
            }
            let is_jpeg = match stream.dict.get(b"Filter").ok() {
                Some(lopdf::Object::Name(n)) => n == b"DCTDecode",
                Some(lopdf::Object::Array(a)) => a.iter().any(|o| {
                    matches!(o, lopdf::Object::Name(n) if n == b"DCTDecode")
                }),
                _ => false,
            };
            if is_jpeg && !stream.content.is_empty() {
                out.push(stream.content.clone());
            }
        }
        out
    }

    /// Reconhece o texto de UM arquivo de imagem via Windows.Media.Ocr.
    fn ocr_image_file(path: &Path) -> Result<String, String> {
        let path_h = HSTRING::from(path.to_string_lossy().as_ref());
        let file = StorageFile::GetFileFromPathAsync(&path_h)
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|e| e.to_string())?;
        let stream = file
            .OpenAsync(FileAccessMode::Read)
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|e| e.to_string())?;
        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|e| e.to_string())?;
        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|e| e.to_string())?;
        // usa os idiomas de OCR instalados no perfil do usuário
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|e| format!("OCR indisponível (pacote de idioma?): {e}"))?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|e| e.to_string())?;
        let text = result.Text().map_err(|e| e.to_string())?;
        Ok(text.to_string_lossy())
    }

    /// Texto OCR desta página (junta as imagens da página). `None` quando não
    /// há imagem JPEG ou o OCR não devolveu nada útil.
    pub fn page_text(path: &Path, page: u32) -> Option<String> {
        let mut text = String::new();
        for (i, jpeg) in page_jpegs(path, page).into_iter().enumerate() {
            // o OCR trabalha com ARQUIVO (StorageFile) — imagem vai e volta
            // por um temp; o custo é desprezível para páginas de scan
            let tmp = std::env::temp_dir()
                .join(format!("localai-ocr-{}-{}-{}.jpg", std::process::id(), page, i));
            if std::fs::write(&tmp, &jpeg).is_err() {
                continue;
            }
            let r = ocr_image_file(&tmp);
            let _ = std::fs::remove_file(&tmp);
            if let Ok(t) = r {
                if !t.trim().is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t.trim());
                }
            }
        }
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

/// Linux/macOS: sem OCR nativo — páginas de scan ficam vazias (a UI avisa
/// "não encontrei texto" em vez de fingir que leu).
#[cfg(not(windows))]
pub fn page_text(_path: &std::path::Path, _page: u32) -> Option<String> {
    None
}

#[cfg(windows)]
pub use imp::page_text;
