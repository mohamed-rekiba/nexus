//! Where the player is: the screens they came through, each with the game they selected on it.
//!
//! The stack is the one record of the player's way through the screens. Back pops one visit
//! and shows the one under it as the player left it, with the same game selected. Themes keep
//! no screen of their own: they show the top visit and send what the player asked for.
//!
//! A running game is not a screen. Starting or ending a game leaves the stack as it is.

use nexus_api::GameKey;

use crate::app::Route;

/// A screen on the stack, with the game the player selected on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visit {
    pub route: Route,
    pub selected: Option<GameKey>,
}

impl Visit {
    fn new(route: Route) -> Self {
        let selected = route.game().cloned();
        Self { route, selected }
    }
}

/// The screens the player came through, Home first. Never empty: Home stays at the bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stack {
    visits: Vec<Visit>,
}

impl Default for Stack {
    fn default() -> Self {
        Self { visits: vec![Visit::new(Route::Home)] }
    }
}

impl Stack {
    /// The screen on top.
    pub fn route(&self) -> &Route {
        &self.top().route
    }

    /// Every visit, Home first.
    #[cfg(test)]
    pub fn visits(&self) -> &[Visit] {
        &self.visits
    }

    /// Back has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        self.visits.len() > 1
    }

    /// Opens `route`:
    /// - Home goes back to the bottom of the stack.
    /// - A screen already on the stack goes back to its visit, as the player left it.
    /// - A plugin's page in place of the plugin's page on top.
    /// - Any other screen goes on top.
    ///
    /// A game's Game Hub selects the game on the visit under it too: Back comes to the game.
    pub fn open(&mut self, route: Route) {
        if let Some(at) = self.visits.iter().rposition(|v| v.route == route) {
            self.visits.truncate(at + 1);
            return;
        }
        if matches!(route, Route::Plugin(_)) && matches!(self.route(), Route::Plugin(_)) {
            self.visits.pop();
        }
        if let Route::GameDetail(game) = &route {
            self.top_mut().selected = Some(game.clone());
        }
        self.visits.push(Visit::new(route));
    }

    /// Back to the visit under the top one. False on Home, the bottom.
    pub fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.visits.pop();
        true
    }

    /// Selects `game` on the screen on top.
    pub fn select(&mut self, game: GameKey) {
        self.top_mut().selected = Some(game);
    }

    /// The game selected on the screen on top.
    pub fn selected(&self) -> Option<&GameKey> {
        self.top().selected.as_ref()
    }

    /// The game selected on the newest visit of `route`, if it is on the stack.
    pub fn selected_on(&self, route: &Route) -> Option<&GameKey> {
        self.visits.iter().rev().find(|v| &v.route == route).and_then(|v| v.selected.as_ref())
    }

    /// Sets the game selected on the newest visit of `route`, if it is on the stack. Home
    /// always is.
    pub fn select_on(&mut self, route: &Route, game: Option<GameKey>) {
        if let Some(visit) = self.visits.iter_mut().rev().find(|v| &v.route == route) {
            visit.selected = game;
        }
    }

    /// The game is gone: its screens close, and no screen keeps it selected.
    pub fn forget_game(&mut self, game: &GameKey) {
        self.retain(|route| route.game() != Some(game));
        for visit in &mut self.visits {
            if visit.selected.as_ref() == Some(game) {
                visit.selected = None;
            }
        }
    }

    /// The screen is gone, such as the Catalog when no catalog is left: every visit of it closes.
    pub fn close(&mut self, route: &Route) {
        self.retain(|r| r != route);
    }

    /// The game whose Game Hub the view shows: the one on top (its Hub or its settings), else
    /// the newest Hub under the screens on top, else the game selected nearest the top.
    pub fn detail_game(&self) -> Option<&GameKey> {
        self.route()
            .game()
            .or_else(|| {
                self.visits.iter().rev().find_map(|v| match &v.route {
                    Route::GameDetail(game) => Some(game),
                    _ => None,
                })
            })
            .or_else(|| self.visits.iter().rev().find_map(|v| v.selected.as_ref()))
    }

    /// Keeps the visits whose route passes `keep`, and Home at the bottom. Two visits of one
    /// screen that end up next to each other become one, the newer.
    fn retain(&mut self, keep: impl Fn(&Route) -> bool) {
        let mut visits: Vec<Visit> = Vec::with_capacity(self.visits.len());
        for (i, visit) in self.visits.drain(..).enumerate() {
            if i > 0 && !keep(&visit.route) {
                continue;
            }
            if visits.last().is_some_and(|last| last.route == visit.route) {
                visits.pop();
            }
            visits.push(visit);
        }
        self.visits = visits;
    }

    fn top(&self) -> &Visit {
        self.visits.last().expect("Home stays at the bottom")
    }

    fn top_mut(&mut self) -> &mut Visit {
        self.visits.last_mut().expect("Home stays at the bottom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(key: &str) -> GameKey {
        key.parse().unwrap()
    }

    fn routes(stack: &Stack) -> Vec<Route> {
        stack.visits().iter().map(|v| v.route.clone()).collect()
    }

    fn hub(key: &str) -> Route {
        Route::GameDetail(game(key))
    }

    #[test]
    fn it_starts_on_home_and_back_there_does_nothing() {
        let mut stack = Stack::default();
        assert_eq!(stack.route(), &Route::Home);
        assert!(!stack.can_go_back());
        assert!(!stack.back());
        assert_eq!(routes(&stack), [Route::Home]);
    }

    #[test]
    fn back_from_a_game_hub_comes_to_the_library_on_the_game() {
        let mut stack = Stack::default();
        stack.open(Route::Library);
        stack.select(game("emu-a:G0"));
        stack.open(hub("emu-a:G1"));

        assert!(stack.back());

        assert_eq!(stack.route(), &Route::Library);
        assert_eq!(stack.selected(), Some(&game("emu-a:G1")), "the game whose Hub it was");
    }

    #[test]
    fn each_screen_keeps_its_own_selection() {
        let mut stack = Stack::default();
        stack.select(game("emu-a:G0"));
        stack.open(Route::Library);
        stack.select(game("emu-a:G1"));

        assert_eq!(stack.selected_on(&Route::Home), Some(&game("emu-a:G0")));
        assert_eq!(stack.selected_on(&Route::Library), Some(&game("emu-a:G1")));
        stack.back();
        assert_eq!(stack.selected(), Some(&game("emu-a:G0")), "Home as the player left it");
        assert_eq!(stack.selected_on(&Route::Library), None, "the Library is closed");
    }

    #[test]
    fn home_s_selection_changes_under_other_screens() {
        let mut stack = Stack::default();
        stack.open(Route::Library);
        stack.select(game("emu-a:G1"));

        stack.select_on(&Route::Home, Some(game("emu-a:G0")));
        stack.select_on(&Route::Catalog, Some(game("emu-a:G2")));

        assert_eq!(stack.selected(), Some(&game("emu-a:G1")), "the Library's stays");
        assert_eq!(stack.selected_on(&Route::Home), Some(&game("emu-a:G0")));
        assert_eq!(stack.selected_on(&Route::Catalog), None, "the Catalog isn't open");
    }

    #[test]
    fn opening_a_screen_on_the_stack_goes_back_to_its_visit() {
        let mut stack = Stack::default();
        stack.open(Route::Library);
        stack.select(game("emu-a:G1"));
        stack.open(hub("emu-a:G1"));
        stack.open(Route::Settings);

        stack.open(Route::Library);

        assert_eq!(routes(&stack), [Route::Home, Route::Library]);
        assert_eq!(stack.selected(), Some(&game("emu-a:G1")));
    }

    #[test]
    fn home_goes_back_to_the_bottom() {
        let mut stack = Stack::default();
        stack.open(Route::Library);
        stack.open(hub("emu-a:G0"));

        stack.open(Route::Home);

        assert_eq!(routes(&stack), [Route::Home]);
    }

    #[test]
    fn a_plugin_s_page_takes_the_place_of_the_one_on_top() {
        let a = Route::Plugin("emu-a".parse().unwrap());
        let b = Route::Plugin("emu-b".parse().unwrap());
        let mut stack = Stack::default();
        stack.open(Route::Emulators);
        stack.open(a);
        stack.open(b.clone());

        assert_eq!(routes(&stack), [Route::Home, Route::Emulators, b]);
    }

    #[test]
    fn a_forgotten_game_closes_its_screens_and_its_selections() {
        let mut stack = Stack::default();
        stack.open(Route::Library);
        stack.open(hub("emu-a:G0"));
        stack.open(Route::GameSettings(game("emu-a:G0")));

        stack.forget_game(&game("emu-a:G0"));

        assert_eq!(routes(&stack), [Route::Home, Route::Library]);
        assert_eq!(stack.selected(), None, "the Library selects nothing");
    }

    #[test]
    fn a_closed_screen_leaves_no_two_visits_of_one_screen_together() {
        let mut stack = Stack::default();
        stack.open(Route::Downloads);
        stack.open(Route::Catalog);
        stack.open(Route::Settings);
        stack.close(&Route::Settings);
        stack.open(Route::Downloads);
        assert_eq!(routes(&stack), [Route::Home, Route::Downloads], "Downloads was already open");

        let mut stack = Stack::default();
        stack.open(Route::Catalog);
        stack.open(hub("emu-a:G0"));
        stack.open(Route::Catalog);
        assert_eq!(routes(&stack), [Route::Home, Route::Catalog]);

        let mut stack = Stack::default();
        stack.open(Route::Catalog);
        stack.close(&Route::Catalog);
        assert_eq!(routes(&stack), [Route::Home], "Home stays");
    }

    #[test]
    fn the_hub_s_game_is_the_one_on_top_else_under_the_top_else_the_selected_one() {
        let mut stack = Stack::default();
        assert_eq!(stack.detail_game(), None);
        stack.select(game("emu-a:G0"));
        assert_eq!(stack.detail_game(), Some(&game("emu-a:G0")), "the selected game");
        stack.open(hub("emu-a:G1"));
        stack.open(Route::Settings);
        assert_eq!(stack.detail_game(), Some(&game("emu-a:G1")), "the Hub under Settings");
        stack.open(Route::GameSettings(game("emu-a:G2")));
        assert_eq!(stack.detail_game(), Some(&game("emu-a:G2")), "the game whose settings show");
    }
}
