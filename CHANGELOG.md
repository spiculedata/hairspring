# Changelog

## Unreleased

### Changed

- `hairspring confirm <run-id> --json` now prints a JSON array with one entry per failure of the run. It used to print
  a single object. `hairspring confirm <failure-id> --json` and `hairspring confirm <bundle> --json` still print one
  object. Scripts that read the output for a run id need to read the first entry of the array, or loop over it.
