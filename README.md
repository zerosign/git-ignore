# git-ignore

This is a personal technical study and a small utility I built to explore what's possible with high performance systems programming in Rust using native ecosystem library in Rust. 
It specifically looks at how to use [redb](https://github.com/cberner/redb) for storing data and [gix (gitoxide)](https://github.com/Byron/gitoxide) for handlin ag Git repositories 
without the usual overhead.

> [!WARNING]
> This is a **pet project** of mine. I'm using it to learn and experiment with optimizations, so it's definitely **not** meant for production use or 
> anything mission critical so please use it at your own risk!

## Why build this?

I wanted to see how far I could push the performance of a simple CLI task fetching and managing `.gitignore` files. Usually, tools like this either call 
the `git` command directly or walk through thousands of files on your disk. I wanted to see if I could do better by going deeper into how these libraries 
work especially native Rust library like `gix`. I found out that `gix` already efficient  enough for read path by doing everything using mmap I guess 
and it also do parallelization using `rayon` in some parts.

I'm using `redb` as simple key value db also experimenting whether for simple use case it's stable enough or not. I'm storing the template string directly 
into redb keyed by its lowercase namespaced template name (ex: `${namespace}/${gitignore-template-name/language}`). I'm using multiple redb table in here just to keep 
it simple :'). 

- `templates` : the key is lowercase namespaced template name and value is the template content.

- `template_oids`: the key is same as above but the content is oid of the git file commit. This is being used for checking whether 
   there is an update or not when traversing the git diff walks for that template.

- `metadata` : the key is `last_commit_hash:${source}` and the value is worktree head oid. This is to check whether our pinned worktree is changed or not 
  (whether the remote head points to new oid or not). if yes then we traverse walk the diff of the git from current oid to remote oid.


## Assumptions

I made a few assumptions about what makes a CLI tool fast on Linux, and then I tested them:

1.  **Reduce number of syscalls**: I assume that reducing syscall is quite important here because of cli tool like this 
    are heavily dealing with rapid teardown during each cli invocations. Having an efficient program that do less syscall 
    seems to be good number i guess.

2.  **Static Linking**: I built the tool with `musl` so it's a completely standalone file. It's a bit larger, but it doesn't have to 
    look for libraries on your system when it starts up.

3.  **Memory Mapping**: I relied on the OS to handle the heavy lifting of reading the database file into memory, which seems a 
    lot more efficient for this kind of work.

### What I measured (on my machine)

> [!NOTE]
> Measured using `strace -c` on static `x86_64-unknown-linux-musl` profiling builds tested using the remote origin `default` (`https://github.com/github/gitignore`).

These are the results I got using `strace -c` on my own Linux setup with the default remote origin. Your results might be different, but they give a good idea of the footprint:


| Task | Command | Syscall Count | Notes |
| :--- | :--- | :--- | :--- |
| **List Everything** | `git-ignore list` | **39** | Ultra-fast zero-copy `rkyv` memory map (down from 96). |
| **Patch .gitignore** | `git-ignore patch Node` | **173** | Parses and merges managed template blocks. |
| **System & DB Info** | `git-ignore info` | **304** | Sources, git HEAD, table counts, and SHA256 checks. |
| **Fresh Upstream Sync** | `git-ignore update -f` | **3,663** | Full git fetch, tree diffing, and paged redb storage. |


*NOTE: A simple "Hello World" in some languages can take over 300 syscalls just to say hello!*

## Main Features

- **Template Registry**: You can pull templates from multiple places at once like the standard GitHub ones plus your own personal repository.
- **Template Diff Mechanism**: I built very simple and stupid diff/patch logic that uses `git-ignore-start/end` markers in your `.gitignore`. It "blindly" do simple diff on which parts 
  it "owns" so it can update them without messing up your own custom rules. I might revise this later on in the future.
- **Template Change Tracker Mechanism**: It only downloads from upstream what has actually changed since the last time you ran it.

## How to play with it

### What you need
- Rust (I recommend Nightly for the ultimate static builds)
- `just` (for running the build commands)

### Just the basics
```bash
cargo build --release
```

```bash
just release-static
```

## TODO 

- [x] Use `usage-rs` replacing `argh` so that I don't need to always update shell completions manually (dynamic generation for Fish, Bash, and Zsh).
- [x] Replace the FST with a zero-copy `rkyv` artifact (`templates.rkyv`) for sub-2ms template listing without opening the database.
- [x] Refactor the codebase into modular command handlers (`update`, `list`, `patch`, `info`, `completion`, `compact`).
- [x] Simplify and streamline error handling into modular domain errors (`StorageError`, `SyncError`, `ConfigError`).
- [ ] Separate Read and Write flows: use `redb::ReadOnlyDatabase` for read-only commands (`patch`, `info`) while using `redb::Database` for mutating operations (`update`, `compact`) to reduce syscalls (`mmap` allocation & `futex` teardown).

## License
MIT - Copyright (c) 2026 zerosign
