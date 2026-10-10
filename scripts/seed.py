#!/usr/bin/env python3
"""Seed the chess puzzle cache from the Lichess puzzle database.

Streams database.lichess.org/lichess_db_puzzle.csv.bz2, keeps the popular
puzzles, buckets them by rating, and writes up to CACHE_PER_BUCKET puzzles
per 100-ELO bucket to the cache JSONL.

Lichess DB quirk: FEN is the position BEFORE the opponent's last move and
Moves[0] is that opponent move — so the stored record applies Moves[0] to
the FEN and the solution starts at Moves[1], matching the shape the panel
gets from the /api/puzzle/next endpoint.
"""
import csv
import io
import json
import os
import sys
import urllib.request

CACHE = os.path.expanduser("~/.local/state/omarchy/chesspuzzles/cache.jsonl")
BUCKET = 100
CACHE_PER_BUCKET = 30
MAX_RATING = 2600
MIN_PLAYS = 200
MIN_POPULARITY = 80
MAX_DEVIATION = 90

FILES = {"pw", "nw", "bw", "rw", "qw", "kw", "pb", "nb", "bb", "rb", "qb", "kb"}


def fen_board(fen):
    board = [""] * 64
    for r, row in enumerate(fen.split(" ")[0].split("/")):
        f = 0
        for ch in row:
            if ch.isdigit():
                f += int(ch)
            else:
                board[r * 8 + f] = ch.lower() + ("w" if ch.isupper() else "b")
                f += 1
    return board


def sq(s):
    return (8 - int(s[1])) * 8 + (ord(s[0]) - 97)


def apply_uci(board, uci):
    frm, to = sq(uci[0:2]), sq(uci[2:4])
    promo = uci[4] if len(uci) > 4 else ""
    piece = board[frm]
    nxt = list(board)
    nxt[to] = piece
    nxt[frm] = ""
    if piece and piece[0] == "p" and frm % 8 != to % 8 and board[to] == "":
        nxt[(frm // 8) * 8 + to % 8] = ""  # en passant
    if piece and piece[1] == "k" and abs(frm % 8 - to % 8) == 2:  # castling
        row = frm // 8
        if to % 8 == 6:
            nxt[row * 8 + 5] = nxt[row * 8 + 7]
            nxt[row * 8 + 7] = ""
        else:
            nxt[row * 8 + 3] = nxt[row * 8 + 0]
            nxt[row * 8 + 0] = ""
    if promo:
        nxt[to] = promo + piece[1]
    return nxt


def convert(fen, moves):
    board = fen_board(fen)
    board = apply_uci(board, moves[0])
    # Rebuild the FEN board half from the applied board; the turn flips.
    rows = []
    for r in range(8):
        row, run = "", 0
        for f in range(8):
            p = board[r * 8 + f]
            if p == "":
                run += 1
            else:
                if run:
                    row += str(run)
                    run = 0
                ch = p[0].upper() if p[1] == "w" else p[0]
                row += ch
        if run:
            row += str(run)
        rows.append(row)
    turn = "b" if fen.split(" ")[1] == "w" else "w"
    new_fen = "/".join(rows) + " " + turn + " - - 0 1"
    return new_fen, board


def main():
    os.makedirs(os.path.dirname(CACHE), exist_ok=True)
    buckets = {}
    seen = set()
    if os.path.exists(CACHE):
        with open(CACHE) as f:
            for line in f:
                try:
                    seen.add(json.loads(line)["id"])
                except Exception:
                    pass

    url = "https://database.lichess.org/lichess_db_puzzle.csv.zst"
    stream = os.popen(
        "curl -fsSL --max-time 900 '" + url + "' | zstd -dc", "r")
    reader = csv.reader(stream)
    header = next(reader)
    assert header[0] == "PuzzleId", header[:3]

    wanted = list(range(6, 26))
    kept = 0
    for row in reader:
        if len(row) < 8:
            continue
        pid, fen, moves, rating, dev, pop, plays, themes = row[:8]
        try:
            rating = int(rating)
        except ValueError:
            continue
        if not (400 <= rating <= MAX_RATING):
            continue
        if int(plays) < MIN_PLAYS or int(pop) < MIN_POPULARITY or int(dev) > MAX_DEVIATION:
            continue
        bucket = rating // BUCKET
        if len(buckets.get(bucket, ())) >= CACHE_PER_BUCKET or pid in seen:
            continue
        move_list = moves.split()
        if len(move_list) < 2:
            continue
        try:
            new_fen, board = convert(fen, move_list)
        except Exception:
            continue
        if not any(board):
            continue
        record = {
            "id": pid,
            "fen": new_fen,
            "rating": rating,
            "themes": [t for t in themes.split() if t][:3],
            "solution": move_list[1:],
        }
        buckets.setdefault(bucket, []).append(record)
        seen.add(pid)
        kept += 1
        if kept % 200 == 0:
            print(f"kept {kept} across {len(buckets)} buckets", flush=True)
            if all(len(buckets.get(b, ())) >= CACHE_PER_BUCKET for b in wanted):
                print("all buckets full", flush=True)
                break

    with open(CACHE, "a") as out:
        for records in buckets.values():
            for r in records:
                out.write(json.dumps(r, separators=(",", ":")) + "\n")
    print(f"wrote {kept} puzzles to {CACHE}")
    stream.close()


if __name__ == "__main__":
    main()
