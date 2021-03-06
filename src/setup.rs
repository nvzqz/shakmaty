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

use square::Square;
use bitboard::Bitboard;
use attacks;
use types::{Color, Role, Pockets, RemainingChecks};
use board::Board;

use option_filter::OptionFilterExt;

/// A not necessarily legal position.
pub trait Setup {
    fn board(&self) -> &Board;
    fn pockets(&self) -> Option<&Pockets>;
    fn turn(&self) -> Color;
    fn castling_rights(&self) -> Bitboard;
    fn ep_square(&self) -> Option<Square>;
    fn remaining_checks(&self) -> Option<&RemainingChecks>;
    fn halfmove_clock(&self) -> u32;
    fn fullmoves(&self) -> u32;

    fn us(&self) -> Bitboard {
        self.board().by_color(self.turn())
    }

    fn our(&self, role: Role) -> Bitboard {
        self.us() & self.board().by_role(role)
    }

    fn them(&self) -> Bitboard {
        self.board().by_color(!self.turn())
    }

    fn their(&self, role: Role) -> Bitboard {
        self.them() & self.board().by_role(role)
    }
}

/// `KingSide` (O-O) or `QueenSide` (O-O-O).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CastlingSide {
    KingSide = 0,
    QueenSide = 1,
}

impl CastlingSide {
    pub fn king_to(&self, color: Color) -> Square {
        match *self {
            CastlingSide::KingSide => color.fold(Square::G1, Square::G8),
            CastlingSide::QueenSide => color.fold(Square::C1, Square::C8),
        }
    }

    pub fn rook_to(&self, color: Color) -> Square {
        match *self {
            CastlingSide::KingSide => color.fold(Square::F1, Square::F8),
            CastlingSide::QueenSide => color.fold(Square::D1, Square::D8),
        }
    }
}

pub struct SwapTurn<S: Setup>(pub S);

impl<S: Setup> Setup for SwapTurn<S> {
    fn turn(&self) -> Color {
        !self.0.turn()
    }

    fn board(&self) -> &Board { self.0.board() }
    fn pockets(&self) -> Option<&Pockets> { self.0.pockets() }
    fn castling_rights(&self) -> Bitboard { self.0.castling_rights() }
    fn ep_square(&self) -> Option<Square> { self.0.ep_square() }
    fn remaining_checks(&self) -> Option<&RemainingChecks> { self.0.remaining_checks() }
    fn halfmove_clock(&self) -> u32 { self.0.halfmove_clock() }
    fn fullmoves(&self) -> u32 { self.0.fullmoves() }
}

#[derive(Clone, Debug)]
pub struct Castling {
    rook: [Option<Square>; 4],
    path: [Bitboard; 4],
}

impl Castling {
    pub fn empty() -> Castling {
        Castling {
            rook: [None; 4],
            path: [Bitboard(0); 4],
        }
    }

    pub fn default() -> Castling {
        Castling {
            rook: [
                Some(Square::H8), // black short
                Some(Square::A8), // black long
                Some(Square::H1), // white short
                Some(Square::A1), // white long
            ],
            path: [
                Bitboard(0x6000_0000_0000_0000), // black short
                Bitboard(0x0e00_0000_0000_0000), // black long
                Bitboard(0x0000_0000_0000_0060), // white short
                Bitboard(0x0000_0000_0000_000e), // white long
            ]
        }
    }

    pub fn from_setup(setup: &Setup) -> Result<Castling, Castling> {
        let mut castling = Castling::empty();

        let castling_rights = setup.castling_rights();
        let rooks = castling_rights & setup.board().rooks();

        for color in &[Color::Black, Color::White] {
            if let Some(king) = setup.board().king_of(*color) {
                if king.file() == 0 || king.file() == 7 || king.rank() != color.fold(0, 7) {
                    continue;
                }

                let side = rooks & setup.board().by_color(*color) &
                           Bitboard::relative_rank(*color, 0);

                if let Some(a_side) = side.first().filter(|rook| rook.file() < king.file()) {
                    let rto = CastlingSide::QueenSide.rook_to(*color);
                    let kto = CastlingSide::QueenSide.king_to(*color);
                    let idx = *color as usize * 2 + CastlingSide::QueenSide as usize;
                    castling.rook[idx] = Some(a_side);
                    castling.path[idx] = attacks::between(king, a_side)
                                        .with(rto).with(kto).without(king).without(a_side);
                }

                if let Some(h_side) = side.last().filter(|rook| king.file() < rook.file()) {
                    let rto = CastlingSide::KingSide.rook_to(*color);
                    let kto = CastlingSide::KingSide.king_to(*color);
                    let idx = *color as usize * 2 + CastlingSide::KingSide as usize;
                    castling.rook[idx] = Some(h_side);
                    castling.path[idx] = attacks::between(king, h_side)
                                        .with(rto).with(kto).without(king).without(h_side);
                }
            }
        }

        if castling.castling_rights() == castling_rights {
            Ok(castling)
        } else {
            Err(castling)
        }
    }

    pub fn discard_rook(&mut self, square: Square) {
        self.rook[0] = self.rook[0].filter(|sq| *sq != square);
        self.rook[1] = self.rook[1].filter(|sq| *sq != square);
        self.rook[2] = self.rook[2].filter(|sq| *sq != square);
        self.rook[3] = self.rook[3].filter(|sq| *sq != square);
    }

    pub fn discard_side(&mut self, color: Color) {
        let idx = color as usize * 2;
        unsafe {
            *self.rook.get_unchecked_mut(idx) = None;
            *self.rook.get_unchecked_mut(idx + 1) = None;
        }
    }

    #[inline]
    pub fn rook(&self, color: Color, side: CastlingSide) -> Option<Square> {
        unsafe { *self.rook.get_unchecked(2 * color as usize + side as usize) }
    }

    #[inline]
    pub fn path(&self, color: Color, side: CastlingSide) -> Bitboard {
        unsafe { *self.path.get_unchecked(2 * color as usize + side as usize) }
    }

    pub fn castling_rights(&self) -> Bitboard {
        let mut mask = Bitboard(0);
        mask.extend(self.rook[0]);
        mask.extend(self.rook[1]);
        mask.extend(self.rook[2]);
        mask.extend(self.rook[3]);
        mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct _AssertObjectSafe(Box<Setup>);
}
