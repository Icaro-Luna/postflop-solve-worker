# postflop-solve-worker

Worker genérico e sem estado para rodar CFR solves de poker via
[`postflop-solver`](https://github.com/b-inary/postflop-solver) (Rust). Não
contém nenhum dado de jogo específico — só o binário e uma esteira que lê um
lote de "pedidos de solve" (JSON) de um repositório de dados (privado,
externo a este), resolve cada um, e devolve o resultado bruto pro mesmo
repositório.

Este repositório é público de propósito: minutos de GitHub Actions são
ilimitados para repositórios públicos, o que o torna um worker de CFR grátis.
Nada do que passa por aqui identifica estratégia, ranges reais ou contexto de
jogo — só spots genéricos de poker (board, ranges, tamanhos de aposta).

## Protocolo da fila

O repositório de dados (configurado via input `data_repo` do workflow) deve
ter:

```
queue/
  pending/
    <id>.json   # pedido: {"id": "...", "payload": {<schema do postflop_cli>}}
  done/
    <id>.json   # resultado: {"id": "...", "raw_output": {...}} ou {"id": "...", "error": "..."}
```

`payload` é exatamente o JSON que o `postflop_cli` espera no stdin — ver
[`solver_native/postflop_cli/src/main.rs`](solver_native/postflop_cli/src/main.rs)
para o schema completo (`oop_range`, `ip_range`, `board`, `starting_pot`,
`effective_stack`, `flop_bets`, `turn_bets`, `river_bets`, `raise_size`,
`hero_is_oop`, `max_iterations`, `target_exploitability_pct`, e opcionais
`hero_hand`/`rake_rate`/`rake_cap`/`facing_bet`/`hero_bet`).

O workflow [`solve-batch.yml`](.github/workflows/solve-batch.yml):

1. Builda o `postflop_cli` (com cache do toolchain Rust).
2. Faz checkout do `data_repo` usando um token com permissão de conteúdo
   nesse repo (`secrets.DATA_REPO_TOKEN`).
3. Pega até `batch_size` itens de `queue/pending/` (com suporte a
   `shard_index`/`shard_count` pra rodar vários jobs em paralelo sem
   sobreposição).
4. Resolve cada um com o binário.
5. Move os itens processados: some de `pending/`, aparece em `done/`.
6. Commita e empurra o resultado de volta pro `data_repo`.

## Licença

O `postflop-solver` upstream é AGPL-3.0. Este wrapper, ao vinculá-lo, segue a
mesma licença — ver [LICENSE](LICENSE). Como este repositório é público, o
código-fonte já está disponível, satisfazendo a cláusula de rede da AGPL.
