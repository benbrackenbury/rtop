# rtop

A process viewer that still works when the terminal is a phone.

Over SSH, the on-screen keyboard covers the bottom third of the display and leaves a short screen. rtop uses one status line there and gives the rest to the process list. On a 24-line terminal it adds cpu and memory meters. On a tall window it adds a per-core grid and the extra columns.

```
cargo run --release
```

`j` `k` or the arrows move. `/` filters. `c` and `m` sort by cpu and memory. `t` toggles the process tree. `x` kills (`t` or enter sends term, `9` sends kill). `z` pauses. `?` lists the keys. `q` quits.
