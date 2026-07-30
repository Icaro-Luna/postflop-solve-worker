//! postflop_cli — wrapper JSON fino sobre b-inary/postflop-solver.
//!
//! Lê um JSON do stdin descrevendo o spot, resolve com Discounted CFR e
//! escreve um JSON no stdout com as frequências de ação (média ponderada do
//! range do herói no nó relevante), exploitability e EV.
//!
//! Contrato de entrada (stdin):
//! {
//!   "oop_range": "AA:1.0,KK:1.0,...",   // range out-of-position (PioSOLVER-like)
//!   "ip_range":  "AA:1.0,...",          // range in-position
//!   "board":     "Kh7c2s",              // 3 (flop), 4 (turn) ou 5 (river) cartas
//!   "starting_pot": 550,                // chips (convenção do pipeline: 1 BB = 100 chips)
//!   "effective_stack": 9725,            // chips
//!   "flop_bets": "33%, 75%",            // bet sizes no flop (ambos jogadores)
//!   "turn_bets": "66%",
//!   "river_bets": "75%",
//!   "raise_size": "60%",                // sizing de raise
//!   "hero_is_oop": false,               // herói é OOP? define qual nó é lido
//!   "max_iterations": 1000,
//!   "target_exploitability_pct": 0.5    // alvo como % do pote
//! }
//!
//! Contrato de saída (stdout): ver struct `Output`.
//!
//! Em erro: escreve {"error": "..."} no stdout e sai com código 1.

use postflop_solver::*;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;

mod export;
mod binary_format;

#[derive(Deserialize)]
struct Input {
    oop_range: String,
    ip_range: String,
    board: String,
    starting_pot: i32,
    effective_stack: i32,
    #[serde(default = "default_flop_bets")]
    flop_bets: String,
    #[serde(default = "default_turn_bets")]
    turn_bets: String,
    #[serde(default = "default_river_bets")]
    river_bets: String,
    #[serde(default = "default_raise")]
    raise_size: String,
    #[serde(default)]
    hero_is_oop: bool,
    #[serde(default = "default_max_iter")]
    max_iterations: u32,
    #[serde(default = "default_target")]
    target_exploitability_pct: f32,
    /// Mão específica do herói (ex: "AhKs"). Quando presente, a saída inclui
    /// `hand_strategy` e `hand_evs_chips` para ESTA mão (não a média do range)
    /// — base da métrica de EV-loss offline (SPEC-027).
    #[serde(default)]
    hero_hand: String,
    /// Rake (SPEC-028): fração do pote (ex.: 0.05) e cap em CHIPS. Default 0
    /// preserva o contrato antigo. Microstakes sem rake superestima calls
    /// marginais — o wrapper Python envia 5%/cap 10BB por default.
    #[serde(default)]
    rake_rate: f64,
    #[serde(default)]
    rake_cap: f64,
    /// Aposta ENFRENTADA pelo herói nesta street, em chips (0 = ninguém
    /// apostou). Navega a árvore até o nó pós-aposta (sizing mais próximo),
    /// onde fold/call/raise existem — sem isto o CLI lia sempre o nó inicial
    /// da street e respondia o nó errado em spots facing-bet (SPEC-028).
    #[serde(default)]
    facing_bet: f64,
    /// Aposta que o PRÓPRIO herói fez nesta street, em chips (0 = não
    /// apostou). Quando > 0 e facing_bet > hero_bet, modela o nó bet→raise:
    /// o herói apostou e o vilão raisou — antes esses spots eram pulados
    /// pela régua de EV-loss (hero_ja_apostou_nao_modelado).
    #[serde(default)]
    hero_bet: f64,
}

fn default_flop_bets() -> String { "33%, 75%".to_string() }
fn default_turn_bets() -> String { "66%".to_string() }
fn default_river_bets() -> String { "75%".to_string() }
fn default_raise() -> String { "60%".to_string() }
fn default_max_iter() -> u32 { 100 }
fn default_target() -> f32 { 0.5 }

#[derive(Serialize)]
struct OutAction {
    kind: String,   // "fold" | "check" | "call" | "bet" | "raise" | "allin"
    amount: i32,    // chips (0 para fold/check/call)
}

#[derive(Serialize)]
struct Output {
    player: usize,              // nó lido: 0 = OOP, 1 = IP
    actions: Vec<OutAction>,
    frequencies: Vec<f32>,      // alinhado com `actions`, soma ~1.0
    exploitability_pct: f32,    // % do pote inicial
    avg_ev_chips: f32,          // EV médio do range do herói no nó (chips)
    starting_pot: i32,
    effective_stack: i32,
    iterations: u32,
    /// Estratégia para a mão específica do herói (alinhada com `actions`).
    /// Presente só quando o input traz `hero_hand` e ela existe no range.
    #[serde(skip_serializing_if = "Option::is_none")]
    hand_strategy: Option<Vec<f32>>,
    /// EV por ação (chips) para a mão específica do herói.
    #[serde(skip_serializing_if = "Option::is_none")]
    hand_evs_chips: Option<Vec<f32>>,
}

fn run(input: Input, export_path: Option<PathBuf>) -> Result<Output, String> {
    // ── Parse do board ────────────────────────────────────────────────────
    let board = input.board.trim();
    if board.len() % 2 != 0 {
        return Err(format!("board com comprimento ímpar: {board:?}"));
    }
    let cards: Vec<String> = board
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect();
    let n = cards.len();
    if !(3..=5).contains(&n) {
        return Err(format!("board precisa ter 3, 4 ou 5 cartas, recebi {n}"));
    }

    let flop_str = format!("{}{}{}", cards[0], cards[1], cards[2]);
    let flop = flop_from_str(&flop_str).map_err(|e| format!("flop inválido {flop_str:?}: {e}"))?;
    let turn = if n >= 4 {
        card_from_str(&cards[3]).map_err(|e| format!("turn inválido {:?}: {e}", cards[3]))?
    } else {
        NOT_DEALT
    };
    let river = if n >= 5 {
        card_from_str(&cards[4]).map_err(|e| format!("river inválido {:?}: {e}", cards[4]))?
    } else {
        NOT_DEALT
    };
    let initial_state = match n {
        3 => BoardState::Flop,
        4 => BoardState::Turn,
        _ => BoardState::River,
    };

    // ── Ranges ────────────────────────────────────────────────────────────
    let oop = input
        .oop_range
        .parse::<Range>()
        .map_err(|e| format!("oop_range inválido: {e}"))?;
    let ip = input
        .ip_range
        .parse::<Range>()
        .map_err(|e| format!("ip_range inválido: {e}"))?;

    let card_config = CardConfig {
        range: [oop, ip],
        flop,
        turn,
        river,
    };

    // ── Bet sizes ─────────────────────────────────────────────────────────
    let flop_bs = BetSizeOptions::try_from((input.flop_bets.as_str(), input.raise_size.as_str()))
        .map_err(|e| format!("flop_bets inválido: {e}"))?;
    let turn_bs = BetSizeOptions::try_from((input.turn_bets.as_str(), input.raise_size.as_str()))
        .map_err(|e| format!("turn_bets inválido: {e}"))?;
    let river_bs = BetSizeOptions::try_from((input.river_bets.as_str(), input.raise_size.as_str()))
        .map_err(|e| format!("river_bets inválido: {e}"))?;

    let tree_config = TreeConfig {
        initial_state,
        starting_pot: input.starting_pot,
        effective_stack: input.effective_stack,
        rake_rate: input.rake_rate,
        rake_cap: input.rake_cap,
        flop_bet_sizes: [flop_bs.clone(), flop_bs],
        turn_bet_sizes: [turn_bs.clone(), turn_bs],
        river_bet_sizes: [river_bs.clone(), river_bs],
        turn_donk_sizes: None,
        river_donk_sizes: None,
        add_allin_threshold: 1.5,
        force_allin_threshold: 0.15,
        merging_threshold: 0.1,
    };

    let action_tree =
        ActionTree::new(tree_config).map_err(|e| format!("falha ao construir ActionTree: {e}"))?;
    let mut game = PostFlopGame::with_config(card_config, action_tree)
        .map_err(|e| format!("falha ao construir PostFlopGame: {e}"))?;

    // ── Solve ─────────────────────────────────────────────────────────────
    // compressão habilitada: reduz memória ~4x (essencial para uso real-time).
    // false = sem compressão (precisão máxima mas pode alocar vários GB).
    game.allocate_memory(true);
    let target = input.starting_pot as f32 * (input.target_exploitability_pct / 100.0);
    let exploitability = solve(&mut game, input.max_iterations, target, false);
    let exploitability_pct = if input.starting_pot > 0 {
        exploitability / input.starting_pot as f32 * 100.0
    } else {
        exploitability
    };

    // ── Navega até o nó do herói ──────────────────────────────────────────
    // Raiz: OOP age primeiro. Se herói é IP, navegamos o "Check" do OOP
    // (caso mais comum coberto pelo MVP — espelha o TexasSolverBackend).
    game.cache_normalized_weights();

    // Helper: joga a ação agressiva (Bet/AllIn) de valor mais próximo do alvo.
    fn play_closest_bet(game: &mut PostFlopGame, target: f64) -> Result<(), String> {
        let acts = game.available_actions();
        let mut best: Option<(usize, f64)> = None;
        for (i, a) in acts.iter().enumerate() {
            let amt = match a {
                Action::Bet(x) | Action::Raise(x) | Action::AllIn(x) => *x as f64,
                _ => continue,
            };
            let d = (amt - target).abs();
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        match best {
            Some((i, _)) => {
                game.play(i);
                game.cache_normalized_weights();
                Ok(())
            }
            None => Err("facing_bet: nó sem ação agressiva para navegar".to_string()),
        }
    }
    fn play_check(game: &mut PostFlopGame) -> Result<(), String> {
        let acts = game.available_actions();
        match acts.iter().position(|a| matches!(a, Action::Check)) {
            Some(i) => {
                game.play(i);
                game.cache_normalized_weights();
                Ok(())
            }
            None => Err("nó sem Check para navegar".to_string()),
        }
    }

    if input.hero_bet > 0.0 && input.facing_bet > input.hero_bet {
        // Nó bet→raise: o herói apostou e o vilão raisou.
        if input.hero_is_oop {
            // OOP apostou (lead na raiz da street), IP raisou.
            play_closest_bet(&mut game, input.hero_bet)?;
            play_closest_bet(&mut game, input.facing_bet)?;
        } else {
            // IP apostou após check do OOP; OOP check-raisou.
            play_check(&mut game)?;
            play_closest_bet(&mut game, input.hero_bet)?;
            play_closest_bet(&mut game, input.facing_bet)?;
        }
    } else if input.facing_bet > 0.0 {
        if input.hero_is_oop {
            // OOP enfrenta aposta: OOP deu check, IP apostou.
            play_check(&mut game)?;
            play_closest_bet(&mut game, input.facing_bet)?;
        } else {
            // IP enfrenta aposta: OOP apostou (donk/lead) na raiz.
            play_closest_bet(&mut game, input.facing_bet)?;
        }
    } else if !input.hero_is_oop {
        // Sem aposta: herói IP age após o Check do OOP (comportamento original).
        let _ = play_check(&mut game); // raiz sem Check (inesperado): lê a raiz
    }

    // ── Lê estratégia no nó atual ─────────────────────────────────────────
    let player = game.current_player();
    let actions = game.available_actions();
    let private = game.private_cards(player);
    let weights = game.normalized_weights(player);
    let strategy = game.strategy(); // len = n_hands * n_actions
    let n_hands = private.len();
    let n_actions = actions.len();

    if n_hands == 0 || n_actions == 0 {
        return Err("nó sem mãos ou sem ações disponíveis".to_string());
    }

    let total_w: f32 = weights.iter().sum();
    let mut frequencies = vec![0f32; n_actions];
    for (a, freq) in frequencies.iter_mut().enumerate() {
        let mut s = 0f32;
        for h in 0..n_hands {
            s += strategy[h + a * n_hands] * weights[h];
        }
        *freq = if total_w > 0.0 { s / total_w } else { 0.0 };
    }

    // EV médio do range do herói (chips)
    let ev = game.expected_values(player);
    let avg_ev_chips = compute_average(&ev, weights);

    // ── Mão específica do herói (SPEC-027: EV-loss offline) ──────────────
    // Localiza a mão no vetor de private cards e extrai a estratégia e o EV
    // POR AÇÃO para ela — `expected_values_detail` tem o mesmo layout
    // [h + a * n_hands] do `strategy`.
    let mut hand_strategy: Option<Vec<f32>> = None;
    let mut hand_evs_chips: Option<Vec<f32>> = None;
    let hh = input.hero_hand.trim();
    if hh.len() == 4 {
        let c1 = card_from_str(&hh[0..2]).map_err(|e| format!("hero_hand inválida {hh:?}: {e}"))?;
        let c2 = card_from_str(&hh[2..4]).map_err(|e| format!("hero_hand inválida {hh:?}: {e}"))?;
        let idx = private
            .iter()
            .position(|&(a, b)| (a == c1 && b == c2) || (a == c2 && b == c1));
        if let Some(h) = idx {
            let ev_detail = game.expected_values_detail(player);
            let mut strat_h = Vec::with_capacity(n_actions);
            let mut evs_h = Vec::with_capacity(n_actions);
            for a in 0..n_actions {
                strat_h.push(strategy[h + a * n_hands]);
                evs_h.push(ev_detail[h + a * n_hands]);
            }
            hand_strategy = Some(strat_h);
            hand_evs_chips = Some(evs_h);
        }
        // Mão fora do range do herói (ex.: range simplificado não a contém):
        // segue sem os campos — o consumidor decide o fallback.
    } else if !hh.is_empty() {
        return Err(format!("hero_hand deve ter 4 chars (ex. AhKs), recebi {hh:?}"));
    }

    let out_actions: Vec<OutAction> = actions
        .iter()
        .map(|a| {
            let (kind, amount) = match a {
                Action::Fold => ("fold", 0),
                Action::Check => ("check", 0),
                Action::Call => ("call", 0),
                Action::Bet(x) => ("bet", *x),
                Action::Raise(x) => ("raise", *x),
                Action::AllIn(x) => ("allin", *x),
                _ => ("none", 0),
            };
            OutAction { kind: kind.to_string(), amount }
        })
        .collect();

    let output = Output {
        player,
        actions: out_actions,
        frequencies,
        exploitability_pct,
        avg_ev_chips,
        starting_pot: input.starting_pot,
        effective_stack: input.effective_stack,
        iterations: input.max_iterations,
        hand_strategy,
        hand_evs_chips,
    };

    if let Some(export_path) = export_path {
        let tree_version = "v7-flopturnriverrich";
        let range_premise_version = "dev";
        let strategy_data = export::export_game(&mut game);
        binary_format::write_file(
            &strategy_data,
            tree_version,
            range_premise_version,
            exploitability_pct,
            input.max_iterations,
            &export_path,
        )
        .map_err(|e| format!("export: {e}"))?;
    }

    Ok(output)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut export_path: Option<PathBuf> = None;
    let mut export_requested = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--export-path" => {
                if i + 1 < args.len() {
                    export_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--export" => {
                export_requested = true;
            }
            _ => {}
        }
        i += 1;
    }
    if export_requested && export_path.is_none() {
        eprintln!("erro: --export requer --export-path <arquivo>");
        std::process::exit(1);
    }
    if !export_requested {
        export_path = None;
    }

    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("erro ao ler stdin: {e}");
        println!("{}", serde_json::json!({"error": format!("stdin: {e}")}));
        std::process::exit(1);
    }

    let input: Input = match serde_json::from_str(&buf) {
        Ok(v) => v,
        Err(e) => {
            println!("{}", serde_json::json!({"error": format!("JSON inválido: {e}")}));
            std::process::exit(1);
        }
    };

    match run(input, export_path) {
        Ok(out) => {
            println!("{}", serde_json::to_string(&out).unwrap());
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "error": e }));
            std::process::exit(1);
        }
    }
}
