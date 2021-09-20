# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.2.1] - 2021-09-20


### Added
- Getter for `scheduler::PodIdentity` fields ([#215]).

[#215]: https://github.com/stackabletech/operator-rs/pull/215

## [0.2.0] - 2021-09-17


### Added
- Extracted the versioning support for up and downgrades from operators ([#211]).
- Added traits to access generic operator versions ([#211]).
- Added init_status method that uses the status default ([#211]).
- Implement StickyScheduler with two pod placement strategies and history stored as K8S status field. ([#210])

### Changed
- `BREAKING`: Changed `Conditions` trait return value to not optional ([#211]). 

[#211]: https://github.com/stackabletech/operator-rs/pull/211
[#210]: https://github.com/stackabletech/operator-rs/pull/210

## 0.1.0 - 2021-09-01

### Added

- Initial release