# Avisos de terceiros (Third-Party Notices)

O LocalAI Studio é distribuído sob a licença MIT (ver `LICENSE`) e
redistribui/depende dos componentes abaixo. Os textos completos das licenças
estão nos repositórios de cada projeto.

## Binários redistribuídos

### llama.cpp (llama-server e bibliotecas ggml)
- Licença: MIT
- Copyright (c) 2023-2024 The ggml authors / Georgi Gerganov
- https://github.com/ggml-org/llama.cpp
- Os instaladores do LocalAI Studio incluem binários oficiais do llama.cpp
  (pasta `binaries/`), baixados dos releases do projeto sem modificação.

## Dependências (Rust)

| Crate | Licença |
|---|---|
| tauri / tauri-build | MIT OR Apache-2.0 |
| serde / serde_json | MIT OR Apache-2.0 |
| sysinfo | MIT |
| ureq | MIT OR Apache-2.0 |
| rfd | MIT |
| raw-cpuid | MIT |

## Dependências (JavaScript/TypeScript)

| Pacote | Licença |
|---|---|
| @tauri-apps/api / @tauri-apps/cli | MIT OR Apache-2.0 |
| marked | MIT |
| dompurify | Apache-2.0 OR MPL-2.0 |
| vite | MIT |
| typescript | Apache-2.0 |

## Serviços

- A busca e o download de modelos usam a API pública do Hugging Face
  (https://huggingface.co). Os modelos baixados têm licenças próprias,
  indicadas na página de cada repositório.
