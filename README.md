# git-ignore

This is a personal technical study and a small utility I built to explore what's possible with high performance systems programming in Rust. It specifically looks at how to use [redb](https://github.com/cberner/redb) for storing data and [gix (gitoxide)](https://github.com/Byron/gitoxide) for handling Git repositories without the usual overhead.

> [!WARNING]
> This is a **pet project** of mine. I'm using it to learn and experiment with optimizations, so it's definitely **not** meant for production use or anything mission critical. Use it at your own risk!

## Why build this?

I wanted to see how far I could push the performance of a simple CLI task fetching and managing `.gitignore` files. Usually, tools like this either call the `git` command directly or walk through thousands of files on your disk. I wanted to see if I could do better by going deeper into how these libraries work.

### My journey with `gix` (Gitoxide)
Instead of just "running git," I'm using `gix` to talk directly to the Git Object Database. 
*   **What I learned**: By performing a **bare clone**, I don't have to wait for the computer to write thousands of tiny files to my hard drive just to read one `.gitignore` template. I can just traverse the repository's "tree" in memory and pull the exact content I need.
*   **The tradeoff**: It's definitely more complex than just running a shell command! You have to handle OIDs and tree structures yourself, which was a great learning experience.

### My journey with `redb`
Once the templates are fetched, I save them in a local `redb` database.
*   **What I learned**: Using memory mapped B-Trees feels like magic. It lets the tool access templates almost instantly without the usual `open()` and `read()` overhead for every single file. It also means the tool works great offline once you've synced it.
*   **The tradeoff**: Honestly, using a database for a few text files is probably "over engineering," but it was the perfect way to test how `redb` handles this kind of workload and ensures the tool always performs the same way, no matter how many templates you have.

## How I looked at optimization

I made a few guesses about what makes a CLI tool fast on Linux, and then I tested them:

1.  **Cutting down System Calls**: I assumed that every time the program has to talk to the Linux kernel, it slows down a bit. So, I tried to keep those "conversations" to a minimum.
2.  **Static Linking**: I built the tool with `musl` so it's a completely standalone file. It's a bit larger, but it doesn't have to look for libraries on your system when it starts up.
3.  **Memory Mapping**: I relied on the OS to handle the heavy lifting of reading the database file into memory, which seems a lot more efficient for this kind of work.

### What I measured (on my machine)
These are the results I got using `strace -c` on my own Linux setup. Your results might be different, but they give a good idea of the footprint:

| Task | Command | Syscall Count | Notes |
| :--- | :--- | :--- | :--- |
| **Check Version** | `git-ignore -v` | **89** | Mostly just starting the process. |
| **List Everything** | `git-ignore -l` | **96** | Quick look at the index. |
| **Get a Template** | `git-ignore Rust` | **111** | Reading directly from memory. |
| **Update a File** | `git-ignore -p Node` | **121** | Parsing and merging blocks. |

*Just for fun: A simple "Hello World" in some languages can take over 300 syscalls just to say hello!*

## Main Features
- **Mix and Match**: You can pull templates from multiple places at once like the standard GitHub ones plus your own personal repository.
- **Smart Merging**: I built a "Patcher" that uses `git-ignore-start/end` markers in your `.gitignore`. It knows which parts it "owns" so it can update them without messing up your own custom rules.
- **Fast Updates**: It only downloads what has actually changed since the last time you ran it.

## How to play with it

### What you need
- Rust (I recommend Nightly for the ultimate static builds)
- `just` (for running the build commands)

### Just the basics
```bash
cargo build --release
```

### The "I want it fast" build
```bash
just release-static
```

## License
MIT - Copyright (c) 2026 zerosign
