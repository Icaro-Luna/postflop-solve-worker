use postflop_solver::{Action, PostFlopGame};

pub struct StrategyData {
    pub node_path: Vec<usize>,
    pub player: usize,
    pub board_state: u8,
    pub actions: Vec<Action>,
    pub num_hands: usize,
    pub strategy: Vec<f32>,
    pub ev: Vec<f32>,
}

pub fn export_game(game: &mut PostFlopGame) -> Vec<StrategyData> {
    let mut results = Vec::new();
    collect_from_root(game, &[], &mut results);
    results
}

fn navigate(game: &mut PostFlopGame, path: &[usize]) {
    game.back_to_root();
    for &action_idx in path {
        if game.is_terminal_node() {
            return;
        }
        game.play(action_idx);
        skip_chance_nodes(game);
    }
}

fn skip_chance_nodes(game: &mut PostFlopGame) {
    while game.is_chance_node() && !game.is_terminal_node() {
        let cards = game.possible_cards();
        if cards != 0 {
            game.play(cards.trailing_zeros() as usize);
        } else {
            break;
        }
        game.cache_normalized_weights();
    }
}

fn collect_from_root(
    game: &mut PostFlopGame,
    path: &[usize],
    results: &mut Vec<StrategyData>,
) {
    navigate(game, path);

    if game.is_terminal_node() {
        return;
    }

    skip_chance_nodes(game);

    if game.is_terminal_node() {
        return;
    }

    game.cache_normalized_weights();

    let actions = game.available_actions();
    let n_actions = actions.len();
    if n_actions == 0 {
        return;
    }

    let player = game.current_player();
    let n_hands = game.private_cards(player).len();
    let strategy = game.strategy();
    let ev_detail = game.expected_values_detail(player);

    let board = game.current_board();
    let board_state: u8 = match board.len() {
        0..=3 => 0,
        4 => 1,
        _ => 2,
    };

    results.push(StrategyData {
        node_path: path.to_vec(),
        player,
        board_state,
        actions: actions.clone(),
        num_hands: n_hands,
        strategy,
        ev: ev_detail,
    });

    for i in 0..n_actions {
        let mut child_path = path.to_vec();
        child_path.push(i);
        collect_from_root(game, &child_path, results);
    }
}
