import { describe, expect, it } from "vitest";
import { ThinkFilter, contentText } from "../../llama";

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
