# Avaliação de lançamento — TaylorAI Studio

> Diagnóstico feito em 2026-07-03 sobre a v0.1.11. Este arquivo é o plano de
> lançamento: cada item vira ✅ quando implementado.
> **Status: quase tudo implementado na v0.2.0 (mesmo dia).**

**Veredito geral:** a base técnica é sólida — o auto-tuner com racional
explicado é um diferencial real, o gerenciamento do llama-server é limpo e o CI
multi-plataforma funciona. O que separava o app de um lançamento público era
ele estar calibrado para uma máquina específica (Ryzen 5 5500U + Vega 7) e
lacunas de UX básicas de chat.

**Licença escolhida: MIT** (decidido em 2026-07-03). AGPL só se pagaria se
fôssemos copiar código do Jan/KoboldCpp (ambos AGPL-3.0); a maior fonte de
reuso é MIT (llama.cpp e a WebUI nova do llama-server, Tauri, GPT4All), então
MIT maximiza adoção sem perder nada.

---

## P0 — Bloqueadores de lançamento

- [x] **Generalizar o tuner para qualquer hardware.** *(v0.2.0)* Nome da GPU
  detectado via `--list-devices`, heurística iGPU (UMA) vs dedicada, textos do
  racional condicionais ao hardware, avisos do 5500U/RAM 16+4 removidos,
  título de janela genérico, pastas de modelos genéricas (LM Studio em
  qualquer unidade + `~/TaylorAI/models`).
- [x] **LICENSE (MIT) + THIRD_PARTY_NOTICES.** *(v0.2.0)*
- [x] **No-think robusto (Qwen 3.5/3.6, Gemma) — ESSENCIAL.** *(v0.2.0)*
  `reasoning_budget: 0` + `enable_thinking` por request (já validado nos Qwen
  3.x) **+ fallback client-side**: `ThinkFilter` em `src/llama.ts` detecta
  `<think>`/`<thinking>` cru no início do stream (finetunes com template
  custom), redireciona para o canal de pensamento até a tag fechar, tolera tag
  partida entre chunks e loga na aba Logs quando atua. Gemma base não pensa
  (toggle é no-op); finetunes com raciocínio destilado caem no fallback.
  **Pendente de teste em modelo real → testar com um Qwen e um finetune.**
- [x] **System prompt persistido** (localStorage), junto com o toggle de
  reasoning. *(v0.2.0)*
- [x] **Markdown no chat** (marked + DOMPurify) + botão copiar (copia a fonte
  crua). *(v0.2.0)*
- [x] **Contador de contexto** no topo (`ctx usados/limite` via
  `stream_options.include_usage`, fica vermelho acima de 80%). *(v0.2.0)*

## P1 — Importantes, não bloqueantes

- [x] **Downloader de modelos (HuggingFace).** *(v0.2.0)* Aba "Baixar
  modelos": busca (filter=gguf, ordenado por downloads), lista de .gguf por
  quant com tamanho, download com progresso/cancelamento (ureq no Rust, sem
  CORS), destino `~/TaylorAI/models` adicionado automaticamente às pastas
  monitoradas + rescan ao concluir.
- [x] **Anexo de imagem no chat** (content parts `image_url` base64,
  thumbnail na conversa, aviso quando o modelo está sem mmproj). *(v0.2.0)*
- [x] **Porta em uso** → o backend escolhe a próxima livre (até +50) e o
  front usa a porta real retornada. *(v0.2.0)*
- [x] **SmartScreen documentado** no README (sem certificado por ora).
- [x] **Conversas em arquivo** (`app_data_dir/conversations.json`) com
  migração automática do localStorage. *(v0.2.0)*

## P2 — Menores

- [x] CSP básica no `tauri.conf.json`. *(v0.2.0)*
- [x] `max_tokens` default 4096. *(v0.2.0)*
- [x] Reasoning salvo com a mensagem (restaurado ao reabrir; não volta à
  API). *(v0.2.0)*
- [ ] Auto-update (plugin updater do Tauri) — requer chave de assinatura de
  update; fica para depois do lançamento.
- [ ] Testar builds Linux/macOS em máquina real (hoje são CI-only).
- [ ] Idioma: UI 100% pt-BR define o público como nicho BR. Decisão consciente
  por ora; inglês + pt-BR dobraria o alcance (fica para depois).

## Checklist de validação manual antes do release público

- [ ] Carregar um Qwen 3.x híbrido: box desmarcada → resposta direta sem
  pensamento; box marcada → canal "Pensando" aparece.
- [ ] Carregar um finetune que despeja `<think>` cru → conferir na aba Logs a
  mensagem `[no-think]` e o pensamento indo para a seção recolhível.
- [ ] Baixar um GGUF pequeno pela aba "Baixar modelos" e carregá-lo.
- [ ] Anexar imagem num modelo com mmproj e conferir a resposta.
- [ ] Fechar e reabrir o app: conversas, system prompt e toggle preservados.
- [ ] Abrir um chat longo até o contador de contexto ficar vermelho.

## Decisões registradas

| Decisão | Escolha | Motivo |
|---|---|---|
| Licença | MIT | Reuso relevante é todo MIT; AGPL fecharia portas sem ganho. |
| Reuso de código AGPL (Jan/KoboldCpp) | Não | Só inspiração de UX; reimplementar. |
| i18n | pt-BR por ora | Nicho consciente; revisitar pós-lançamento. |
| Assinatura de código | Sem certificado no lançamento | Custo; documentar workaround SmartScreen. |
