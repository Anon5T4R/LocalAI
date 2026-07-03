# TaylorAI Studio

Um "clone do LM Studio" enxuto, focado em **extrair o máximo de CPUs e iGPUs**
rodando modelos **GGUF** via [llama.cpp](https://github.com/ggml-org/llama.cpp).
O diferencial não é a interface — é o **auto-tuner**: ele detecta o seu
hardware (núcleos, SIMD, RAM, GPU Vulkan e o orçamento de memória dela) e o
metadado do modelo, e escolhe os flags do `llama-server` que rendem mais na
sua máquina — explicando o porquê de cada escolha.

Recursos:

- **Auto-tuner** com racional e avisos por máquina (threads, offload GPU
  total/parcial, KV cache, mlock, flash-attn, speculative decoding).
- **Chat** com markdown, botão copiar, múltiplas conversas persistidas em
  disco, métricas de tok/s e contador de contexto usado.
- **Modo raciocínio opcional** (desligado por padrão): controla o "pensar"
  dos Qwen 3.x / híbridos via `reasoning_budget` por request, com fallback
  que captura `<think>` cru de modelos cujo template ignora o controle
  (finetunes de Gemma/Qwen).
- **Visão (mmproj)**: anexe imagens no chat para modelos multimodais.
- **Baixar modelos** direto do Hugging Face (busca, lista de GGUFs por quant,
  progresso e cancelamento).

> Nasceu afinado num Ryzen 5 5500U + Vega 7 (as heurísticas de iGPU/UMA vêm de
> benchmarks reais nessa classe de máquina), mas as recomendações são
> calculadas a partir do hardware detectado em cada computador.

## Arquitetura

```
┌─────────────────────────────────────────────┐
│  Janela Tauri (WebView2, ~tens de MB de RAM) │
│  ┌─────────────┐   ┌──────────────────────┐  │
│  │  Frontend   │   │  Backend Rust         │  │
│  │  (TS puro)  │←→ │  • detecção de HW     │  │
│  │  chat/UI    │   │  • parser GGUF        │  │
│  └──────┬──────┘   │  • AUTO-TUNER         │  │
│         │          │  • gerência de proc.  │  │
│         │          └──────────┬────────────┘  │
└─────────┼─────────────────────┼───────────────┘
          │ HTTP (OpenAI API)   │ spawn
          ▼                     ▼
   ┌──────────────────────────────────┐
   │  llama-server.exe (b9723, Vulkan)│
   │  backend CPU AVX2 + offload Vega │
   └──────────────────────────────────┘
```

O motor de inferência é o próprio `llama-server` (API compatível com OpenAI).
O app **não reescreve inferência** — ele a orquestra de forma ótima.

## O que o auto-tuner decide (exemplo: Ryzen 5 5500U)

| Flag | Valor no 5500U | Motivo |
|------|----------------|--------|
| `-t` (threads geração) | 6 (núcleos físicos) | Geração é *memory-bound*; usar os 12 threads SMT contende a banda e piora. |
| `-tb` (threads prompt) | 12 (lógicos) | Prompt é *compute-bound* e escala com mais threads. |
| `-ngl` (camadas na GPU) | 0 (padrão) / opcional Vulkan | A Vega compartilha a RAM do sistema: offload ajuda no prompt, nem sempre na geração. |
| `--flash-attn on` | sempre | Reduz RAM do KV cache e acelera. |
| `--mlock` | se couber folgado | Trava o modelo na RAM, evita paginação para o disco. |
| `--cache-type-k/v q8_0` | opcional | Metade da banda/RAM do KV em contextos longos. |
| `-c` (contexto) | 8192 padrão | Maior = mais RAM de KV e geração mais lenta no fim. |

A aba **Ajustes & Auto-tuner** mostra a estimativa de RAM, a linha de comando
final e a justificativa de cada escolha.

## Pré-requisitos (já resolvidos neste setup)

- Node.js + npm
- Rust (toolchain MSVC) — usa o linker do Visual Studio
- WebView2 Runtime (vem no Windows 11)
- Binários do llama.cpp em `src-tauri/binaries/` (build **win-vulkan-x64**)

## Rodar em desenvolvimento

```powershell
npm install
npm run tauri dev
```

## Gerar o executável

```powershell
npm run tauri build
# saída em: <target>/release/bundle/
```

> O `target/` do Rust é redirecionado para fora do OneDrive via
> `src-tauri/.cargo/config.toml` (evita sincronizar GBs de artefatos).

## Builds multi-plataforma (GitHub Actions)

`.github/workflows/release.yml` builda **Windows, Linux e macOS** automaticamente
ao dar push numa tag `vX.Y.Z` (ou manualmente via *Run workflow*). Ele baixa o
runtime do llama.cpp certo por plataforma (Vulkan no Windows/Linux, Metal no
macOS), builda com a action oficial do Tauri e anexa os instaladores ao release.

```bash
git tag v0.1.3 && git push origin v0.1.3   # dispara o CI multi-plataforma
```

> Apenas o build Windows foi validado no hardware-alvo (Ryzen 5 5500U). Linux e
> macOS são gerados pelo CI e ainda não testados em máquina real. Para bumpar o
> runtime do llama.cpp, ajuste `LLAMA_TAG` no workflow.

## Dicas de desempenho (notebooks / APUs)

1. **Na tomada + modo "Melhor desempenho"** — o boost cai muito na bateria.
2. **Banda de memória é o teto físico** da geração: dual-channel pleno (dois
   pentes iguais) faz diferença real de tok/s.
3. **Q4_K_M** costuma ser o melhor custo-benefício de velocidade/qualidade;
   Q6/Q8 dão mais qualidade ao custo de banda.
4. Para contexto longo, ligue o **KV q8_0**.
5. Em iGPU, **offload Vulkan é tudo-ou-nada**: total costuma ganhar do CPU
   puro; parcial costuma perder. Compare com `ngl=0`.

## Modelos

O app procura `.gguf` nas pastas padrão do LM Studio e do cache do Hugging
Face, na pasta `~/TaylorAI/models` (destino dos downloads da aba **Baixar
modelos**) e em qualquer pasta que você adicionar. Modelos com arquivo
`mmproj-*` são multimodais (visão) e o app passa `--mmproj` quando você os
carrega com a visão ligada.

## Instalação no Windows (SmartScreen)

Os instaladores ainda **não são assinados digitalmente** (certificado de
código tem custo). Na primeira execução o SmartScreen pode avisar: clique em
**Mais informações → Executar assim mesmo**. Os binários são gerados pelo CI
público deste repositório — você pode auditar o workflow e os artefatos.

## Licença

MIT — ver [LICENSE](LICENSE). Componentes de terceiros (incluindo os binários
do llama.cpp redistribuídos): [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
