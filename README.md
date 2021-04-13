# spawner2

[![AppVeyor Build Status](https://ci.appveyor.com/api/projects/status/github/klenin/spawner2?svg=true)](https://ci.appveyor.com/project/klenin/spawner2)

[![Build Status](https://travis-ci.org/klenin/spawner2.svg)](https://travis-ci.org/klenin/spawner2)

Crossplatform sandbox for running user submitted code. Designed as a part of [CATS](https://github.com/klenin/cats-judge) contest control system.

### Building
```
git clone git@github.com:klenin/spawner2.git
cd spawner2
cargo build
```

### Installation on UNIX
In order for `spawner2` to work on UNIX you need to run `create_cgroups.sh`  every time after system startup.

### Tests
Use following command to run tests:
```
cargo test -- --test-threads=1
```