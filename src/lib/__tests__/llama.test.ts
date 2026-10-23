import { describe, expect, it, vi } from "vitest";
import {
  ThinkFilter,
  ToolCallAggregator,
  contentText,
  parseToolCallsFromText,
  streamChat,
  type SamplingParams,
  type ToolDef,
} from "../../llama";

describe("contentText", () => {
  // O content multimodal carrega a imagem como data-URL base64 de centenas de
  // KB. Se ela vazasse pro texto, o titulo da conversa e o que e reenviado como
  // historico virariam lixo binario.
  it("descarta as partes de imagem e junta so o texto", () => {
    expect(
      contentText([
        { type: "image_url", image_url: { url: "data:image/png;base64,AAAA" } },
        { type: "text", text: "o que e isso" },
        { type: "text", text: "seja breve" },
      ]),
    ).toBe("o que e isso\nseja breve");
  });

  // Mensagem so com imagem (o app manda "Descreva a imagem." como default, mas
  // nada impede um content sem parte de texto): tem que dar string vazia, nao
  // "undefined" nem estourar.
  it("content so de imagem vira string vazia", () => {
    expect(
      contentText([
        { type: "image_url", image_url: { url: "data:image/png;base64,AAAA" } },
      ]),
    ).toBe("");
  });
});

/** Roda o filtro sobre uma sequencia de chunks e concatena cada canal. */
function run(chunks: string[]) {
  const f = new ThinkFilter();
  let delta = "";
  let reasoning = "";
  const notes: string[] = [];
  for (const c of chunks) {
    const out = f.feed(c);
    delta += out.delta ?? "";
    reasoning += out.reasoning ?? "";
    if (out.note) notes.push(out.note);
  }
  const rest = f.flush();
  delta += rest.delta ?? "";
  reasoning += rest.reasoning ?? "";
  return { delta, reasoning, notes };
}

describe("ThinkFilter", () => {
  // Caso base do fallback: o modelo ignorou reasoning_budget e despejou <think>
  // cru. Sem isso a resposta na tela comeca com o monologo interno.
  it("desvia o miolo de <think> pro canal de pensamento", () => {
    const r = run(["<think>", "vou somar", "</think>", "da 4"]);
    expect(r.delta).toBe("da 4");
    expect(r.reasoning).toBe("vou somar");
    expect(r.notes).toHaveLength(1);
  });

  // O SSE quebra onde quiser: a tag de abertura chega partida entre chunks.
  // Comparar chunk a chunk com "<think>" nao detecta nada e o pensamento
  // inteiro vaza pra resposta.
  it("tag de abertura partida entre chunks ainda e detectada", () => {
    const r = run(["<thi", "nk>", "hmm", "</think>", "ola"]);
    expect(r.delta).toBe("ola");
    expect(r.reasoning).toBe("hmm");
  });

  // Espelho do anterior no fechamento: se a cauda "</thi" nao for segurada, ela
  // sai como pensamento e o "nk>" seguinte vira o comeco da resposta visivel.
  it("tag de fechamento partida entre chunks nao vaza pra resposta", () => {
    const r = run(["<think>razao</thi", "nk>resposta"]);
    expect(r.delta).toBe("resposta");
    expect(r.reasoning).toBe("razao");
  });

  // "<" e prefixo de "<think>": o filtro segura o buffer enquanto a duvida
  // existe. Uma resposta que legitimamente comeca com "<" (codigo, HTML) nao
  // pode ser engolida quando o proximo caractere descarta a hipotese.
  it("texto que so parece tag e liberado inteiro", () => {
    const r = run(["<", "div>oi"]);
    expect(r.delta).toBe("<div>oi");
    expect(r.reasoning).toBe("");
  });

  // Filtrar <think> em qualquer posicao apagaria o meio de uma resposta que
  // FALA sobre a tag. So vale no comeco do stream.
  it("<think> no meio do texto nao e filtrado", () => {
    const r = run(["a tag <think>x</think> serve pra isso"]);
    expect(r.delta).toBe("a tag <think>x</think> serve pra isso");
    expect(r.reasoning).toBe("");
  });

  // Variante <thinking> da mesma familia de finetunes.
  it("reconhece a variante <thinking>", () => {
    const r = run(["<thinking>oi</thinking>tchau"]);
    expect(r.delta).toBe("tchau");
    expect(r.reasoning).toBe("oi");
  });

  // Muitos templates emitem "\n" antes da tag; comparar o buffer cru com
  // startsWith("<think>") falha e o pensamento inteiro vaza.
  it("espaco antes da tag nao impede a deteccao", () => {
    const r = run(["\n  <think>razao</think>resposta"]);
    expect(r.delta).toBe("resposta");
    expect(r.reasoning).toBe("razao");
  });

  // Depois de </think> sobra "\n\n": tem que sumir do inicio da resposta, mas o
  // corte so vale ate a resposta comecar — um trim global comeria a indentacao
  // de blocos de codigo no meio do stream.
  it("apara o branco inicial da resposta mas preserva o do meio", () => {
    const r = run(["<think>x</think>\n\ndef f():", "\n    return 1"]);
    expect(r.delta).toBe("def f():\n    return 1");
  });

  // Stream cortado (usuario apertou parar) com o buffer segurando uma hipotese
  // de tag: o flush precisa devolver esse texto, senao o ultimo pedaco da
  // resposta some da tela.
  it("flush devolve o texto retido quando o stream corta na duvida", () => {
    const r = run(["<thi"]);
    expect(r.delta).toBe("<thi");
  });

  // Corte no meio do pensamento: a sobra vai pro canal de pensamento, nunca
  // pra resposta.
  it("flush no meio do pensamento nao joga a sobra na resposta", () => {
    // corta logo apos "</thi", que estava retido como possivel fechamento
    const r = run(["<think>razao</thi"]);
    expect(r.delta).toBe("");
    expect(r.reasoning).toBe("razao</thi");
  });
});

describe("ToolCallAggregator", () => {
  it("agrega deltas por index (arguments em pedaços)", () => {
    const agg = new ToolCallAggregator();
    agg.feed([
      {
        index: 0,
        id: "call_1",
        function: { name: "save_document", arguments: '{"file' },
      },
    ]);
    agg.feed([
      { index: 0, function: { arguments: 'name":"rel","format":"md"}' } },
    ]);
    const out = agg.drain();
    expect(out).toHaveLength(1);
    expect(out[0].id).toBe("call_1");
    expect(out[0].name).toBe("save_document");
    expect(JSON.parse(out[0].arguments)).toEqual({
      filename: "rel",
      format: "md",
    });
  });

  // Várias chamadas no mesmo turno: cada index é uma chamada; a ordem do
  // output segue o index, não a ordem de chegada dos deltas.
  it("ordena por index e descarta chamadas sem nome", () => {
    const agg = new ToolCallAggregator();
    agg.feed([{ index: 1, id: "c2", function: { name: "b", arguments: "{}" } }]);
    agg.feed([{ index: 0, id: "c1", function: { name: "a", arguments: "{}" } }]);
    agg.feed([{ index: 2, id: "c3" }]); // delta sem name (só arguments)
    const out = agg.drain();
    expect(out.map((c) => c.name)).toEqual(["a", "b"]);
  });

  it("vazio devolve lista vazia", () => {
    expect(new ToolCallAggregator().drain()).toEqual([]);
  });
});

// GARANTIA do loop de ferramentas: o streamChat precisa EMITIR os tool_calls
// agregados dos deltas. Regressão real (v0.11.0): a classe existia mas nunca
// era alimentada — o modelo gerava 770 tokens de tool call, o app descartava
// e nada acontecia (bolha vazia).
describe("streamChat + tool_calls", () => {
  const params: SamplingParams = {
    temperature: 0.7,
    top_p: 0.95,
    top_k: 40,
    min_p: 0.05,
    repeat_penalty: 1.1,
    max_tokens: 100,
  };
  const tool: ToolDef = {
    type: "function",
    function: { name: "save_document", description: "d", parameters: {} },
  };

  it("agrega os deltas e emite o chunk toolCalls antes do done", async () => {
    const sse = [
      'data: {"choices":[{"index":0,"finish_reason":null,"delta":{"role":"assistant","content":""}}]}',
      'data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"save_document","arguments":"{\\"filename\\":"}}]}}]}',
      'data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\\"ata\\",\\"format\\":\\"docx\\"}"}}]}}]}',
      'data: {"choices":[{"index":0,"finish_reason":"tool_calls","delta":{}}]}',
      "data: [DONE]",
      "",
    ].join("\n\n");
    const orig = globalThis.fetch;
    globalThis.fetch = vi.fn(
      async () => new Response(sse, { status: 200 }),
    ) as unknown as typeof fetch;
    try {
      const chunks: Array<{ toolCalls?: unknown; done?: boolean }> = [];
      for await (const c of streamChat(
        1,
        [{ role: "user", content: "x" }],
        params,
        new AbortController().signal,
        false,
        [tool],
      )) {
        chunks.push(c);
      }
      const emitted = chunks.find((c) => c.toolCalls);
      expect(emitted).toBeTruthy();
      expect(emitted!.toolCalls).toEqual([
        {
          id: "call_1",
          name: "save_document",
          arguments: '{"filename":"ata","format":"docx"}',
        },
      ]);
      expect(chunks[chunks.length - 1].done).toBe(true);
    } finally {
      globalThis.fetch = orig;
    }
  });
});

describe("parseToolCallsFromText (fallback de servidor)", () => {
  it("extrai o bloco com name/arguments e limpa o texto", () => {
    const src =
      'Vou criar.\n<tool_call>\n{"name":"save_document","arguments":{"filename":"ata","format":"docx","content":"x"}}\n</tool_call>';
    const { text, calls } = parseToolCallsFromText(src);
    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe("save_document");
    expect(JSON.parse(calls[0].arguments).format).toBe("docx");
    expect(text.trim()).toBe("Vou criar.");
  });

  it("bloco inválido ou sem name fica intacto (não come texto legítimo)", () => {
    const bad = "<tool_call>isso não é json</tool_call>";
    const r1 = parseToolCallsFromText(bad);
    expect(r1.calls).toHaveLength(0);
    expect(r1.text).toBe(bad);
    const noName = '<tool_call>{"arguments":{}}</tool_call>';
    const r2 = parseToolCallsFromText(noName);
    expect(r2.calls).toHaveLength(0);
  });

  it("arguments já em string JSON é preservado", () => {
    const { calls } = parseToolCallsFromText(
      '<tool_call>{"name":"x","arguments":"{\\"a\\":1}"}</tool_call>',
    );
    expect(calls[0].arguments).toBe('{"a":1}');
  });
});
