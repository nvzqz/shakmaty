// This file is part of the shakmaty library.
// Copyright (C) 2017 Niklas Fiekas <niklas.fiekas@backscattering.de>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <http://www.gnu.org/licenses/>.

use attacks;
use board::Board;
use bitboard::Bitboard;
use square::Square;
use types::{Color, White, Black, Role, Piece, Move, Pockets, RemainingChecks};
use setup::{Setup, Castling, CastlingSide, SwapTurn};
use movelist::{MoveList, ArrayVecExt};

use option_filter::OptionFilterExt;

use std::fmt;
use std::error::Error;

/// Outcome of a game.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Outcome {
    Decisive { winner: Color },
    Draw,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match *self {
            Outcome::Decisive { winner: White } => "1-0",
            Outcome::Decisive { winner: Black } => "0-1",
            Outcome::Draw => "1/2-1/2",
        })
    }
}

bitflags! {
    /// Reasons for a [`Setup`] not beeing a legal [`Position`].
    ///
    /// [`Setup`]: trait.Setup.html
    /// [`Position`]: trait.Position.html
    pub struct PositionError: u32 {
        const EMPTY_BOARD = 1;
        const MISSING_KING = 2;
        const TOO_MANY_KINGS = 4;
        const PAWNS_ON_BACKRANK = 8;
        const BAD_CASTLING_RIGHTS = 16;
        const INVALID_EP_SQUARE = 32;
        const OPPOSITE_CHECK = 64;
    }
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "illegal position".fmt(f)
    }
}

impl Error for PositionError {
    fn description(&self) -> &str {
        "illegal position"
    }
}

impl PositionError {
    fn into_result<T>(self, ok: T) -> Result<T, PositionError> {
        if self.is_empty() {
            Ok(ok)
        } else {
            Err(self)
        }
    }
}

/// Error in case of illegal moves.
#[derive(Debug)]
pub struct IllegalMove;

impl fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "illegal move".fmt(f)
    }
}

impl Error for IllegalMove {
    fn description(&self) -> &str {
        "illegal move"
    }
}

/// A legal chess or chess variant position. See [`Chess`] for a concrete
/// implementation.
///
/// [`Chess`]: struct.Chess.html
pub trait Position: Setup {
    /// Set up a position.
    ///
    /// # Errors
    ///
    /// Returns [`PositionError`] if the setup is not legal.
    ///
    /// [`PositionError`]: enum.PositionError.html
    fn from_setup<S: Setup>(setup: &S) -> Result<Self, PositionError> where Self: Sized;

    /// Swap turns. This is sometimes called "playing a null move".
    ///
    /// # Errors
    ///
    /// Returns [`PositionError`] if swapping turns is not legal (usually due
    /// to a check that has to be averted).
    fn swap_turn(self) -> Result<Self, PositionError> where Self: Sized {
        Self::from_setup(&SwapTurn(self))
    }

    /// Generates legal moves.
    fn legals(&self) -> MoveList {
        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals
    }

    /// Collects all legal moves in an existing buffer.
    ///
    /// # Panics
    ///
    /// Panics if `moves` is too full. This can not happen if an empty
    /// [`MoveList`] is passed.
    ///
    /// [`MoveList`]: type.MoveList.html
    fn legal_moves(&self, moves: &mut MoveList);

    /// Generates a subset of legal moves: All piece moves and drops of type
    /// `role` to the square `to`, excluding castling moves.
    ///
    /// # Panics
    ///
    /// Panics if `moves` is too full. This can not happen if an empty
    /// [`MoveList`] is passed.
    ///
    /// [`MoveList`]: type.MoveList.html
    fn san_candidates(&self, role: Role, to: Square, moves: &mut MoveList) {
        self.legal_moves(moves);
        filter_san_candidates(role, to, moves);
    }

    /// Generates castling moves.
    ///
    /// # Panics
    ///
    /// Panics if `moves` is too full. This can not happen if an empty
    /// [`MoveList`] is passed.
    ///
    /// [`MoveList`]: type.MoveList.html
    fn castling_moves(&self, side: CastlingSide, moves: &mut MoveList) {
        self.legal_moves(moves);
        moves.retain(|m| match *m {
            Move::Castle { rook, king } =>
                (rook.file() > king.file()) == (side == CastlingSide::KingSide),
            _ => false
        });
    }

    /// Tests a move for legality.
    fn is_legal(&self, m: &Move) -> bool {
        let mut moves = MoveList::new();
        match *m {
            Move::Normal { role, to, .. } | Move::Put { role, to } =>
                self.san_candidates(role, to, &mut moves),
            Move::EnPassant { to, .. } =>
                self.san_candidates(Role::Pawn, to, &mut moves),
            Move::Castle { king, rook } if king.file() < rook.file() =>
                self.castling_moves(CastlingSide::KingSide, &mut moves),
            Move::Castle { .. } =>
                self.castling_moves(CastlingSide::QueenSide, &mut moves),
        }
        moves.contains(m)
    }

    /// Tests if a move is irreversible.
    ///
    /// In standard chess pawn moves, captures and moves that destroy castling
    /// rights are irreversible.
    fn is_irreversible(&self, m: &Move) -> bool {
        match *m {
            Move::Normal { role: Role::Pawn, .. } |
                Move::Normal { capture: Some(_), .. } |
                Move::Castle { .. } |
                Move::EnPassant { .. } |
                Move::Put { .. } => true,
            Move::Normal { role, from, to, .. } =>
                self.castling_rights().contains(from) ||
                self.castling_rights().contains(to) ||
                (role == Role::King && (self.castling_rights() & Bitboard::relative_rank(self.turn(), 0)).any())
        }
    }

    /// Attacks that a king on `square` would have to deal with.
    fn king_attackers(&self, square: Square, attacker: Color, occupied: Bitboard) -> Bitboard {
        self.board().attacks_to(square, attacker, occupied)
    }

    /// Tests the rare case where moving the rook to the other side during
    /// castling would uncover a rank attack.
    fn castling_uncovers_rank_attack(&self, rook: Square, king_to: Square) -> bool;

    /// Bitboard of pieces giving check.
    fn checkers(&self) -> Bitboard {
        self.our(Role::King).first()
            .map_or(Bitboard(0), |king| self.king_attackers(king, !self.turn(), self.board().occupied()))
    }

    /// Checks if the game is over due to a special variant end condition.
    ///
    /// Note that for example stalemate is not considered a variant-specific
    /// end condition (`is_variant_end()` will return `false`), but it can have
    /// a special [`variant_outcome()`](#tymethod.variant_outcome) in suicide
    /// chess.
    fn is_variant_end(&self) -> bool;

    /// Tests for checkmate.
    fn is_checkmate(&self) -> bool {
        if self.checkers().is_empty() {
            return false;
        }

        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals.is_empty()
    }

    /// Tests for stalemate.
    fn is_stalemate(&self) -> bool {
        if !self.checkers().is_empty() || self.is_variant_end() {
            false
        } else {
            let mut legals = MoveList::new();
            self.legal_moves(&mut legals);
            legals.is_empty()
        }
    }

    /// Tests for insufficient winning material.
    fn is_insufficient_material(&self) -> bool;

    /// Tests if the game is over due to [checkmate](#method.is_checkmate),
    /// [stalemate](#method.is_stalemate),
    /// [insufficient material](#tymethod.is_insufficient_material) or
    /// [variant end](#tymethod.is_variant_end).
    fn is_game_over(&self) -> bool {
        let mut legals = MoveList::new();
        self.legal_moves(&mut legals);
        legals.is_empty() || self.is_insufficient_material()
    }

    /// Tests special variant winning, losing and drawing conditions.
    fn variant_outcome(&self) -> Option<Outcome>;

    /// The outcome of the game, or `None` if the game is not over.
    fn outcome(&self) -> Option<Outcome> {
        self.variant_outcome().or_else(|| {
            if self.is_checkmate() {
                Some(Outcome::Decisive { winner: !self.turn() })
            } else if self.is_stalemate() || self.is_insufficient_material() {
                Some(Outcome::Draw)
            } else {
                None
            }
        })
    }

    /// Plays a move.
    ///
    /// # Errors
    ///
    /// Returns [`IllegalMove`] if the move is not legal in the position.
    ///
    /// [`IllegalMove`]: struct.IllegalMove.html
    fn play(mut self, m: &Move) -> Result<Self, IllegalMove>
        where Self: Sized
    {
        if self.is_legal(m) {
            self.play_unchecked(m);
            Ok(self)
        } else {
            Err(IllegalMove {})
        }
    }

    /// Plays a move. It is the callers responsibility to ensure the move is
    /// legal.
    ///
    /// # Panics
    ///
    /// Illegal moves can corrupt the state of the position and may
    /// (or may not) panic or cause panics on future calls.
    fn play_unchecked(&mut self, m: &Move);
}

/// A standard Chess position.
#[derive(Clone, Debug)]
pub struct Chess {
    board: Board,
    turn: Color,
    castling: Castling,
    ep_square: Option<Square>,
    halfmove_clock: u32,
    fullmoves: u32,
}

impl Default for Chess {
    fn default() -> Chess {
        Chess {
            board: Board::default(),
            turn: White,
            castling: Castling::default(),
            ep_square: None,
            halfmove_clock: 0,
            fullmoves: 1,
        }
    }
}

impl Setup for Chess {
    fn board(&self) -> &Board { &self.board }
    fn pockets(&self) -> Option<&Pockets> { None }
    fn turn(&self) -> Color { self.turn }
    fn castling_rights(&self) -> Bitboard { self.castling.castling_rights() }
    fn ep_square(&self) -> Option<Square> { self.ep_square.filter(|&s| is_relevant_ep(self, s)) }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { None }
    fn halfmove_clock(&self) -> u32 { self.halfmove_clock }
    fn fullmoves(&self) -> u32 { self.fullmoves }
}

impl Position for Chess {
    fn play_unchecked(&mut self, m: &Move) {
        do_move(&mut self.board, &mut self.turn, &mut self.castling,
                &mut self.ep_square, &mut self.halfmove_clock,
                &mut self.fullmoves, m);
    }

    fn from_setup<S: Setup>(setup: &S) -> Result<Chess, PositionError> {
        let (castling, errors) = match Castling::from_setup(setup) {
            Ok(castling) => (castling, PositionError::empty()),
            Err(castling) => (castling, PositionError::BAD_CASTLING_RIGHTS),
        };

        let pos = Chess {
            board: setup.board().clone(),
            turn: setup.turn(),
            castling: castling,
            ep_square: setup.ep_square(),
            halfmove_clock: setup.halfmove_clock(),
            fullmoves: setup.fullmoves(),
        };

        (validate(&pos) | errors).into_result(pos)
    }

    fn castling_uncovers_rank_attack(&self, rook: Square, king_to: Square) -> bool {
        castling_uncovers_rank_attack(self, rook, king_to)
    }

    fn legal_moves(&self, moves: &mut MoveList) {
        let king = self.board().king_of(self.turn()).expect("king in standard chess");

        let has_ep = gen_en_passant(self.board(), self.turn(), self.ep_square, moves);

        let checkers = self.checkers();
        if checkers.is_empty() {
            let target = !self.us();
            gen_non_king(self, target, moves);
            gen_safe_king(self, king, target, moves);
            gen_castling_moves(self, king, CastlingSide::KingSide, moves);
            gen_castling_moves(self, king, CastlingSide::QueenSide, moves);
        } else {
            evasions(self, king, checkers, moves);
        }

        let blockers = slider_blockers(self.board(), self.them(), king);
        if blockers.any() || has_ep {
            moves.swap_retain(|m| is_safe(self, king, m, blockers));
        }
    }

    fn castling_moves(&self, side: CastlingSide, moves: &mut MoveList) {
        let king = self.board().king_of(self.turn()).expect("king in standard chess");
        gen_castling_moves(self, king, side, moves);
    }

    fn san_candidates(&self, role: Role, to: Square, moves: &mut MoveList) {
        let king = self.board().king_of(self.turn()).expect("king in standard chess");
        let checkers = self.checkers();

        if checkers.is_empty() {
            let piece_from = match role {
                Role::Pawn | Role::King => Bitboard(0),
                Role::Knight => attacks::knight_attacks(to),
                Role::Bishop => attacks::bishop_attacks(to, self.board().occupied()),
                Role::Rook => attacks::rook_attacks(to, self.board().occupied()),
                Role::Queen => attacks::queen_attacks(to, self.board().occupied()),
            };

            if !self.us().contains(to) {
                match role {
                    Role::Pawn => gen_pawn_moves(self, Bitboard::from_square(to), moves),
                    Role::King => gen_safe_king(self, king, Bitboard::from_square(to), moves),
                    _ => {}
                }

                assert!(moves.len() + 8 < moves.capacity());

                for from in piece_from & self.our(role) {
                    unsafe {
                        moves.push_unchecked(Move::Normal {
                            role,
                            from,
                            capture: self.board().role_at(to),
                            to,
                            promotion: None,
                        });
                    };
                }
            }
        } else {
            evasions(self, king, checkers, moves);
            filter_san_candidates(role, to, moves);
        }

        let has_ep =
            role == Role::Pawn &&
            Some(to) == self.ep_square &&
            gen_en_passant(self.board(), self.turn(), self.ep_square, moves);

        let blockers = slider_blockers(self.board(), self.them(), king);
        if blockers.any() || has_ep {
            moves.swap_retain(|m| is_safe(self, king, m, blockers));
        }
    }

    fn is_insufficient_material(&self) -> bool {
        if self.board().pawns().any() || self.board().rooks_and_queens().any() {
            return false;
        }

        if self.board().occupied().count() < 3 {
            return true; // single knight or bishop
        }

        if self.board().knights().any() {
            return false; // more than a single knight
        }

        // all bishops on the same color
        if (self.board().bishops() & Bitboard::DARK_SQUARES).is_empty() {
            return true;
        }
        if (self.board().bishops() & Bitboard::LIGHT_SQUARES).is_empty() {
            return true;
        }

        false
    }

    fn is_variant_end(&self) -> bool { false }
    fn variant_outcome(&self) -> Option<Outcome> { None }
}

fn do_move(board: &mut Board,
           turn: &mut Color,
           castling: &mut Castling,
           ep_square: &mut Option<Square>,
           halfmove_clock: &mut u32,
           fullmoves: &mut u32,
           m: &Move) {
    let color = *turn;
    ep_square.take();
    *halfmove_clock = halfmove_clock.saturating_add(1);

    match *m {
        Move::Normal { role, from, capture, to, promotion } => {
            if role == Role::Pawn || capture.is_some() {
                *halfmove_clock = 0;
            }

            if role == Role::Pawn && (from - to == 16 || from - to == -16) {
                *ep_square = from.offset(color.fold(8, -8));
            }

            if role == Role::King {
                castling.discard_side(color);
            } else if role == Role::Rook {
                castling.discard_rook(from);
            }

            if capture == Some(Role::Rook) {
                castling.discard_rook(to);
            }

            let promoted = board.promoted().contains(from) || promotion.is_some();

            board.discard_piece_at(from);
            board.set_piece_at(to, promotion.map_or(role.of(color), |p| p.of(color)), promoted);
        },
        Move::Castle { king, rook } => {
            let rook_to = (if rook - king < 0 { Square::D1 } else { Square::F1 }).combine(rook);
            let king_to = (if rook - king < 0 { Square::C1 } else { Square::G1 }).combine(king);

            board.discard_piece_at(king);
            board.discard_piece_at(rook);
            board.set_piece_at(rook_to, color.rook(), false);
            board.set_piece_at(king_to, color.king(), false);

            castling.discard_side(color);
        },
        Move::EnPassant { from, to } => {
            board.discard_piece_at(to.combine(from)); // captured pawn
            board.remove_piece_at(from).map(|piece| board.set_piece_at(to, piece, false));
            *halfmove_clock = 0;
        },
        Move::Put { role, to } => {
            board.set_piece_at(to, Piece { color, role }, false);
        },
    }

    if color.is_black() {
        *fullmoves = fullmoves.saturating_add(1);
    }

    *turn = !color;
}

fn validate<P: Position>(pos: &P) -> PositionError {
    let mut errors = PositionError::empty();

    if pos.board().occupied().is_empty() {
        errors |= PositionError::EMPTY_BOARD;
    }

    if (pos.board().pawns() & (Bitboard::rank(0) | Bitboard::rank(7))).any() {
        errors |= PositionError::PAWNS_ON_BACKRANK;
    }

    // validate en passant square
    if let Some(ep_square) = pos.ep_square() {
        if !Bitboard::relative_rank(pos.turn(), 5).contains(ep_square) {
            errors |= PositionError::INVALID_EP_SQUARE;
        } else {
            let fifth_rank_sq = ep_square.offset(pos.turn().fold(-8, 8))
                                         .expect("ep square is on sixth rank");

            let seventh_rank_sq  = ep_square.offset(pos.turn().fold(8, -8))
                                            .expect("ep square is on sixth rank");

            // The last move must have been a double pawn push. Check for the
            // presence of that pawn.
            if !pos.their(Role::Pawn).contains(fifth_rank_sq) {
                errors |= PositionError::INVALID_EP_SQUARE;
            }

            if pos.board().occupied().contains(ep_square) || pos.board().occupied().contains(seventh_rank_sq) {
                errors |= PositionError::INVALID_EP_SQUARE;
            }
        }
    }

    for &color in &[White, Black] {
        if pos.board().king_of(color).is_none() {
            errors |= PositionError::MISSING_KING;
        }
    }

    if pos.board().kings().count() > 2 {
        errors |= PositionError::TOO_MANY_KINGS;
    }

    if let Some(their_king) = pos.board().king_of(!pos.turn()) {
        if pos.king_attackers(their_king, pos.turn(), pos.board().occupied()).any() {
            errors |= PositionError::OPPOSITE_CHECK;
        }
    }

    errors
}

fn gen_non_king<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
    gen_pawn_moves(pos, target, moves);
    KnightTag::gen_moves(pos, target, moves);
    BishopTag::gen_moves(pos, target, moves);
    RookTag::gen_moves(pos, target, moves);
    QueenTag::gen_moves(pos, target, moves);
}

fn gen_safe_king<P: Position>(pos: &P, king: Square, target: Bitboard, moves: &mut MoveList) {
    assert!(moves.len() + 8 < moves.capacity());

    for to in attacks::king_attacks(king) & target {
        if pos.board().attacks_to(to, !pos.turn(), pos.board().occupied()).is_empty() {
            unsafe {
                moves.push_unchecked(Move::Normal {
                    role: Role::King,
                    from: king,
                    capture: pos.board().role_at(to),
                    to,
                    promotion: None,
                });
            }
        }
    }
}

fn evasions<P: Position>(pos: &P, king: Square, checkers: Bitboard, moves: &mut MoveList) {
    let sliders = checkers & pos.board().sliders();

    let mut attacked = Bitboard(0);
    for checker in sliders {
        attacked |= attacks::ray(checker, king) ^ checker;
    }

    gen_safe_king(pos, king, !pos.us() & !attacked, moves);

    if let Some(checker) = checkers.single_square() {
        let target = attacks::between(king, checker).with(checker);
        gen_non_king(pos, target, moves);
    }
}

fn gen_castling_moves(pos: &Chess, king: Square, side: CastlingSide, moves: &mut MoveList) {
    if let Some(rook) = pos.castling.rook(pos.turn(), side) {
        let path = pos.castling.path(pos.turn(), side);
        if (path & pos.board().occupied()).any() {
            return;
        }

        let king_to = side.king_to(pos.turn());
        let king_path = attacks::between(king, king_to).with(king_to).with(king);
        for sq in king_path {
            if pos.king_attackers(sq, !pos.turn(), pos.board().occupied() ^ king).any() {
                return;
            }
        }

        if pos.castling_uncovers_rank_attack(rook, king_to) {
            return;
        }

        moves.push(Move::Castle { king, rook });
    }
}

fn castling_uncovers_rank_attack<P: Position>(pos: &P, rook: Square, king_to: Square) -> bool {
    (attacks::rook_attacks(king_to, pos.board().occupied().without(rook)) &
     pos.them() & pos.board().rooks_and_queens() &
     Bitboard::rank(king_to.rank())).any()
}

trait Stepper {
    const ROLE: Role;

    fn attacks(from: Square) -> Bitboard;

    fn gen_moves<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
        assert!(moves.len() + 8 < moves.capacity());

        for from in pos.our(Self::ROLE) {
            for to in Self::attacks(from) & target {
                unsafe {
                    moves.push_unchecked(Move::Normal {
                        role: Self::ROLE,
                        from,
                        capture: pos.board().role_at(to),
                        to,
                        promotion: None
                    });
                }
            }
        }
    }
}

trait Slider {
    const ROLE: Role;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard;

    fn gen_moves<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
        assert!(moves.len() + 28 < moves.capacity());

        for from in pos.our(Self::ROLE) {
            for to in Self::attacks(from, pos.board().occupied()) & target {
                unsafe {
                    moves.push_unchecked(Move::Normal {
                        role: Self::ROLE,
                        from,
                        capture: pos.board().role_at(to),
                        to,
                        promotion: None
                    });
                }
            }
        }
    }
}

enum KnightTag { }
enum BishopTag { }
enum RookTag { }
enum QueenTag { }

impl Stepper for KnightTag {
    const ROLE: Role = Role::Knight;
    fn attacks(from: Square) -> Bitboard {
        attacks::knight_attacks(from)
    }
}

impl Slider for BishopTag {
    const ROLE: Role = Role::Bishop;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard {
        attacks::bishop_attacks(from, occupied)
    }
}

impl Slider for RookTag {
    const ROLE: Role = Role::Rook;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard {
        attacks::rook_attacks(from, occupied)
    }
}

impl Slider for QueenTag {
    const ROLE: Role = Role::Queen;
    fn attacks(from: Square, occupied: Bitboard) -> Bitboard {
        attacks::queen_attacks(from, occupied)
    }
}

fn gen_pawn_moves<P: Position>(pos: &P, target: Bitboard, moves: &mut MoveList) {
    // Due to push_unchecked the safety of this function depends on this
    // assertion.
    assert!(moves.len() + 108 < moves.capacity());

    let seventh = pos.our(Role::Pawn) & Bitboard::relative_rank(pos.turn(), 6);

    for from in pos.our(Role::Pawn) & !seventh {
        for to in attacks::pawn_attacks(pos.turn(), from) & pos.them() & target {
            unsafe {
                moves.push_unchecked(Move::Normal {
                    role: Role::Pawn,
                    from,
                    capture: pos.board().role_at(to),
                    to,
                    promotion: None
                });
            }
        }
    }

    for from in seventh {
        for to in attacks::pawn_attacks(pos.turn(), from) & pos.them() & target {
            unsafe {
                push_promotions(moves, from, to, pos.board().role_at(to));
            }
        }
    }

    let single_moves = pos.our(Role::Pawn).relative_shift(pos.turn(), 8) &
                       !pos.board().occupied();

    let double_moves = single_moves.relative_shift(pos.turn(), 8) &
                       Bitboard::relative_rank(pos.turn(), 3) &
                       !pos.board().occupied();

    for to in single_moves & target & !Bitboard::BACKRANKS {
        if let Some(from) = to.offset(pos.turn().fold(-8, 8)) {
            unsafe {
                moves.push_unchecked(Move::Normal {
                    role: Role::Pawn,
                    from,
                    capture: None,
                    to,
                    promotion: None
                });
            }
        }
    }

    for to in single_moves & target & Bitboard::BACKRANKS {
        if let Some(from) = to.offset(pos.turn().fold(-8, 8)) {
            unsafe {
                push_promotions(moves, from, to, None);
            }
        }
    }

    for to in double_moves & target {
        if let Some(from) = to.offset(pos.turn().fold(-16, 16)) {
            unsafe {
                moves.push_unchecked(Move::Normal {
                    role: Role::Pawn,
                    from,
                    capture: None,
                    to,
                    promotion: None
                });
            }
        }
    }
}

unsafe fn push_promotions(moves: &mut MoveList, from: Square, to: Square, capture: Option<Role>) {
    moves.push_unchecked(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Queen) });
    moves.push_unchecked(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Rook) });
    moves.push_unchecked(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Bishop) });
    moves.push_unchecked(Move::Normal { role: Role::Pawn, from, capture, to, promotion: Some(Role::Knight) });
}

fn is_relevant_ep<P: Position>(pos: &P, ep_square: Square) -> bool {
    let mut moves = MoveList::new();
    gen_en_passant(pos.board(), pos.turn(), Some(ep_square), &mut moves) && {
        moves.clear();
        pos.legal_moves(&mut moves);
        moves.iter().any(|m| match *m {
            Move::EnPassant { to, .. } => to == ep_square,
            _ => false
        })
    }
}

fn gen_en_passant(board: &Board, turn: Color, ep_square: Option<Square>, moves: &mut MoveList) -> bool {
    let mut found = false;

    if let Some(to) = ep_square {
        for from in board.pawns() & board.by_color(turn) & attacks::pawn_attacks(!turn, to) {
            moves.push(Move::EnPassant { from, to });
            found = true;
        }
    }

    found
}

fn slider_blockers(board: &Board, enemy: Bitboard, king: Square) -> Bitboard {
    let snipers = (attacks::rook_attacks(king, Bitboard(0)) & board.rooks_and_queens()) |
                  (attacks::bishop_attacks(king, Bitboard(0)) & board.bishops_and_queens());

    let mut blockers = Bitboard(0);

    for sniper in snipers & enemy {
        let b = attacks::between(king, sniper) & board.occupied();

        if !b.more_than_one() {
            blockers.add_all(b);
        }
    }

    blockers
}

fn is_safe<P: Position>(pos: &P, king: Square, m: &Move, blockers: Bitboard) -> bool {
    match *m {
        Move::Normal { from, to, .. } =>
            !(pos.us() & blockers).contains(from) || attacks::aligned(from, to, king),
        Move::EnPassant { from, to } => {
            let mut occupied = pos.board().occupied();
            occupied.flip(from);
            occupied.flip(to.combine(from)); // captured pawn
            occupied.add(to);

            (attacks::rook_attacks(king, occupied) & pos.them() & pos.board().rooks_and_queens()).is_empty() &
            (attacks::bishop_attacks(king, occupied) & pos.them() & pos.board().bishops_and_queens()).is_empty()
        },
        _ => true,
    }
}

fn filter_san_candidates(role: Role, to: Square, moves: &mut MoveList) {
    moves.retain(|m| match *m {
        Move::Normal { role: r, to: t, .. } | Move::Put { role: r, to: t } =>
            to == t && role == r,
        Move::EnPassant { to: t, .. } => role == Role::Pawn && t == to,
        Move::Castle { .. } => false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use fen::Fen;

    #[cfg(nightly)]
    use test::Bencher;

    struct _AssertObjectSafe(Box<Position>);

    #[test]
    fn test_most_known_legals() {
        let fen = "R6R/3Q4/1Q4Q1/4Q3/2Q4Q/Q4Q2/pp1Q4/kBNN1KB1 w - - 0 1";
        let pos: Chess = fen.parse::<Fen>()
            .expect("valid fen")
            .position()
            .expect("legal position");

        let mut moves = MoveList::new();
        pos.legal_moves(&mut moves);
        assert_eq!(moves.len(), 218);
    }

    #[cfg(nightly)]
    #[bench]
    fn bench_generate_moves(b: &mut Bencher) {
        let fen = "rn1qkb1r/pbp2ppp/1p2p3/3n4/8/2N2NP1/PP1PPPBP/R1BQ1RK1 b kq -";
        let pos: Chess = fen.parse::<Fen>()
            .expect("valid fen")
            .position()
            .expect("legal position");

        b.iter(|| {
            let mut moves = MoveList::new();
            pos.legal_moves(&mut moves);
            assert_eq!(moves.len(), 39);
        });
    }

    #[cfg(nightly)]
    #[bench]
    fn bench_play_unchecked(b: &mut Bencher) {
        let fen = "rn1qkb1r/pbp2ppp/1p2p3/3n4/8/2N2NP1/PP1PPPBP/R1BQ1RK1 b kq -";
        let pos: Chess = fen.parse::<Fen>()
            .expect("valid fen")
            .position()
            .expect("legal position");

        let m = Move::Normal {
            role: Role::Bishop,
            from: Square::F8,
            capture: None,
            to: Square::E7,
            promotion: None,
        };

        b.iter(|| {
            let mut pos = pos.clone();
            pos.play_unchecked(&m);
            assert_eq!(pos.turn(), White);
        });
    }

    #[cfg(nightly)]
    #[bench]
    fn bench_san_candidates(b: &mut Bencher) {
        let fen = "r2q1rk1/pb1nbppp/5n2/1p2p3/3NP3/P1NB4/1P2QPPP/R1BR2K1 w - -";
        let pos: Chess = fen.parse::<Fen>()
            .expect("valid fen")
            .position()
            .expect("legal position");

        b.iter(|| {
            let mut moves = MoveList::new();
            pos.san_candidates(Role::Knight, Square::B5, &mut moves);
            assert_eq!(moves.len(), 2);
        });
    }

    #[test]
    fn test_pinned_san_candidate() {
        let fen = "R2r2k1/6pp/1Np2p2/1p2pP2/4p3/4K3/3r2PP/8 b - - 5 37";
        let pos: Chess = fen.parse::<Fen>()
            .expect("valid fen")
            .position()
            .expect("valid position");

        let mut moves = MoveList::new();
        pos.san_candidates(Role::Rook, Square::D3, &mut moves);

        assert_eq!(moves[0], Move::Normal {
            role: Role::Rook,
            from: Square::D2,
            capture: None,
            to: Square::D3,
            promotion: None,
        });

        assert_eq!(moves.len(), 1);
    }
}
