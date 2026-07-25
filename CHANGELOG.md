# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.2.2](https://github.com/TudorAndrei/amux/compare/4999187bd34402c055024587d9bb9c9b4b47af07..v0.2.2) - 2026-07-25
#### Bug Fixes
- (**daemon**) keep subscriber broadcasts blocking - ([cf717b7](https://github.com/TudorAndrei/amux/commit/cf717b724a71d7be93248c627665253ab8bae1f1)) - TudorAndrei
- (**daemon**) reclaim idle subscriptions - ([aab8858](https://github.com/TudorAndrei/amux/commit/aab8858841183e1154e9c795cb2536a406ca2e20)) - TudorAndrei
- (**daemon**) attach monitor from tmux clients - ([b25286c](https://github.com/TudorAndrei/amux/commit/b25286cf3b04dfaa6d6cd2e9f01431728922bd03)) - TudorAndrei
- (**picker**) reliably switch all tmux sessions - ([daa7e80](https://github.com/TudorAndrei/amux/commit/daa7e8031c5b3c86e53c2cf6133398ee43ebdda7)) - TudorAndrei
- (**state**) protect fallback event storage - ([d1463c6](https://github.com/TudorAndrei/amux/commit/d1463c652e061f38d659d79c645620c663b059e4)) - TudorAndrei
- (**state**) coordinate clear with daemon state - ([4999187](https://github.com/TudorAndrei/amux/commit/4999187bd34402c055024587d9bb9c9b4b47af07)) - TudorAndrei
- (**tmux**) stabilize control subscriptions - ([387e500](https://github.com/TudorAndrei/amux/commit/387e5002b09947fcb8fe1c7578d0791380e3c404)) - TudorAndrei
- (**tmux**) decode escaped control records - ([f688932](https://github.com/TudorAndrei/amux/commit/f688932b024328b634cb9d1d17d4ecff3a1b5886)) - TudorAndrei
#### Tests
- (**daemon**) avoid filesystem subscription fixture - ([7d7d7ec](https://github.com/TudorAndrei/amux/commit/7d7d7ec7957bd5b5ea3f1320a74c98a9f836f463)) - TudorAndrei
- (**tmux**) isolate socket directories - ([72ec5f7](https://github.com/TudorAndrei/amux/commit/72ec5f7e2d573b086aed4ed17d1a78204500e9cb)) - TudorAndrei
- (**tmux**) serialize real server fixtures - ([79ab726](https://github.com/TudorAndrei/amux/commit/79ab7261f8b2c6973ff72066788d174a3836b0e0)) - TudorAndrei
- (**tmux**) cover typed topology fixtures - ([811ad13](https://github.com/TudorAndrei/amux/commit/811ad13c6cb32fd520354eebb7191031b34e4ecb)) - TudorAndrei
#### Refactoring
- (**tmux**) type topology snapshots - ([e4972c7](https://github.com/TudorAndrei/amux/commit/e4972c732b7a63a0c2295cb71baceb9e1551c908)) - TudorAndrei

- - -

## [v0.2.1](https://github.com/TudorAndrei/amux/compare/8cf1622178325fc3a8099b8dd5e37e7afccaf39f..v0.2.1) - 2026-07-24
#### Bug Fixes
- (**picker**) restore ctrl navigation - ([8cf1622](https://github.com/TudorAndrei/amux/commit/8cf1622178325fc3a8099b8dd5e37e7afccaf39f)) - TudorAndrei

- - -

## [v0.2.0](https://github.com/TudorAndrei/amux/compare/b6477d9c74305d70b38cd1f651896d9209fc2f2e..v0.2.0) - 2026-07-24
#### Features
- (**picker**) integrate nucleo session matching - ([b6477d9](https://github.com/TudorAndrei/amux/commit/b6477d9c74305d70b38cd1f651896d9209fc2f2e)) - TudorAndrei
#### Bug Fixes
- (**test**) wait for nucleo snapshots - ([a2d3a20](https://github.com/TudorAndrei/amux/commit/a2d3a20ee4dc54f12eef49e79889b8a380ef2ea7)) - TudorAndrei

- - -

## [v0.1.2](https://github.com/TudorAndrei/amux/compare/a21e93951bc4dbbd64b3141cddc2813b0d57b0da..v0.1.2) - 2026-07-24
#### Bug Fixes
- (**tpm**) install matching native release - ([a21e939](https://github.com/TudorAndrei/amux/commit/a21e93951bc4dbbd64b3141cddc2813b0d57b0da)) - TudorAndrei

- - -

## [v0.1.1](https://github.com/TudorAndrei/amux/compare/42eac7e82f9ba6b8e80e379fe9f8041cbe532759..v0.1.1) - 2026-07-24
#### Bug Fixes
- (**ci**) install Rust lint components with mise - ([7c5108c](https://github.com/TudorAndrei/amux/commit/7c5108ccbd187fd7590a9dfa0fa71ae6e464be4e)) - TudorAndrei
- (**picker**) restore fzf and switch pane targets - ([42eac7e](https://github.com/TudorAndrei/amux/commit/42eac7e82f9ba6b8e80e379fe9f8041cbe532759)) - TudorAndrei
- (**runtime**) migrate hooks and mise tasks - ([74cafd2](https://github.com/TudorAndrei/amux/commit/74cafd28d9891f09caa2a5e2283d674fec5d32bd)) - TudorAndrei

- - -

## [v0.1.0](https://github.com/TudorAndrei/amux/compare/v0.0.0..v0.1.0) - 2026-07-24
#### Features
- replace amux shell runtime with native Rust (#1) - ([7495d0a](https://github.com/TudorAndrei/amux/commit/7495d0a093a1dca1f850852dbd9505261ea0c837)) - Tudor Andrei Dumitrascu
#### Bug Fixes
- (**release**) use Cocogitto v7 bump command - ([d423c35](https://github.com/TudorAndrei/amux/commit/d423c35d6b71a65a0ece478ecdd6f8a56afa8ad4)) - TudorAndrei
- (**test**) bound tmux monitor subscriptions - ([c5a3792](https://github.com/TudorAndrei/amux/commit/c5a379271b80c560a1eaf0039e62399cc253080e)) - TudorAndrei
- (**tmux**) replace legacy status commands - ([ac907aa](https://github.com/TudorAndrei/amux/commit/ac907aa171833db9da05243af088ce182038e88f)) - TudorAndrei

- - -

## [v0.0.0](https://github.com/TudorAndrei/amux/compare/0b49072b43cb0c11c9f1a577b1e6e5f5d7a0ad9f..v0.0.0) - 2026-07-23
#### Features
- (**core**) add hook event sink and state store - ([989feaf](https://github.com/TudorAndrei/amux/commit/989feaf2fdd39728b66a0e1a8bb5aaaf7c7d9e76)) - TudorAndrei
- (**hooks**) add global integrations for supported agents - ([e87e680](https://github.com/TudorAndrei/amux/commit/e87e680c582e2270465a1ce95c1f7421ac629f3c)) - TudorAndrei
- (**tmux**) list all sessions with agent liveness - ([d11d632](https://github.com/TudorAndrei/amux/commit/d11d632e5ca1f62af0da57fdcf41db6678b9f461)) - TudorAndrei
- (**tmux**) add status and attention picker - ([4c24a14](https://github.com/TudorAndrei/amux/commit/4c24a140f936b5db6006d187e2515f1227ccb885)) - TudorAndrei
#### Bug Fixes
- (**core**) return success for empty status - ([81d6733](https://github.com/TudorAndrei/amux/commit/81d673376809b9794555107874bd27e927c6cf7d)) - TudorAndrei
- (**core**) sort state records reliably - ([1b26272](https://github.com/TudorAndrei/amux/commit/1b2627231d64a67eee3b92aa97707aff265d8ec9)) - TudorAndrei
- (**tmux**) keep picker columns aligned while searching - ([6b66fd4](https://github.com/TudorAndrei/amux/commit/6b66fd470d957bd6e6643e78b2304ee26c587882)) - TudorAndrei
- (**tmux**) match picker search on hidden session field - ([bd52a4f](https://github.com/TudorAndrei/amux/commit/bd52a4f4fb191e4d18a24570ef55c32636c839ec)) - TudorAndrei
- (**tmux**) search picker by session name only - ([7d04fad](https://github.com/TudorAndrei/amux/commit/7d04fad29525a225fe9bcaefca827bfff3e8025c)) - TudorAndrei
- (**tmux**) widen popup and align picker columns - ([d12c74f](https://github.com/TudorAndrei/amux/commit/d12c74feaffe5297ef6c78fc19646c19a671c0ce)) - TudorAndrei
- (**tmux**) simplify picker session rows - ([bd59a97](https://github.com/TudorAndrei/amux/commit/bd59a970d836c9d61a8a26dd7699251913a45424)) - TudorAndrei
- (**tmux**) show sessions instead of agent records - ([0d31ee0](https://github.com/TudorAndrei/amux/commit/0d31ee0ca52a963579dd872a8b8641bf249fcc8d)) - TudorAndrei
- (**tmux**) clean old picker binding when key changes - ([ae4c6d5](https://github.com/TudorAndrei/amux/commit/ae4c6d5793c89e366a6d1f667fd2dbcb42114af9)) - TudorAndrei
- (**tmux**) keep picker open without agent state - ([99c5f65](https://github.com/TudorAndrei/amux/commit/99c5f65aeecdf75aa4079330229914bfe6608233)) - TudorAndrei
- prevent state.json from growing unbounded and breaking jq argv - ([79e67d6](https://github.com/TudorAndrei/amux/commit/79e67d6454a91c945be503dd9f9a2c0e19403c3e)) - TudorAndrei
#### Documentation
- (**install**) document and automate global hook setup - ([069e354](https://github.com/TudorAndrei/amux/commit/069e35482c6927ed2860f512879c36d4635b4a97)) - TudorAndrei
- (**readme**) remove dotfiles-local install notes - ([e49043c](https://github.com/TudorAndrei/amux/commit/e49043cb071315e6acb8b83a4d491423880c764f)) - TudorAndrei
- prepare public plugin docs - ([7881b64](https://github.com/TudorAndrei/amux/commit/7881b64bcb4e2274f043d212587d68e6d5d60d11)) - TudorAndrei

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).