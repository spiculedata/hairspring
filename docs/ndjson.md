# NDJSON reads

Every list the REST API serves can also be read as NDJSON (one JSON object per line), and `hairspring results` and
`hairspring show` can print the same. It is meant for scripts: `curl ... | jq`, a line-by-line parser, a
notebook, with no HTML or table scraping.

## Asking for it

Send `Accept: application/x-ndjson`. The header is the only switch (there is no `?format=` parameter).

| List | Items are under this key in the JSON form |
|---|---|
| `GET /api/targets` | `targets` |
| `GET /api/runs` | `runs` |
| `GET /api/failures` | `failures` |
| `GET /api/workers` | `workers` |
| `GET /api/runs/{id}/properties` | `properties` |
| `GET /api/runs/{id}/trials` | `trials` |
| `GET /api/runs/{id}/coverage` | `points` |

The seed strip, single objects (a run, a property, a trial), downloads, and every `POST` ignore the header and answer
as before. The response is JSON unless NDJSON is asked for more strongly than JSON: `application/json, application/x-ndjson`
(a tie), `*/*`, another type, `q=0`, or a malformed header all give JSON, never `406`.

A successful NDJSON response has `Content-Type: application/x-ndjson; charset=utf-8`, `X-Content-Type-Options: nosniff`
and `Cache-Control: no-store`. Authentication, roles, validation, rate limits and page limits are the JSON form's: the
same query parameters (`limit`, `cursor`, filters) apply, a page never holds more than the JSON form's page would, and
an error is a JSON body with its usual status (`401`, `403`, `404`, `400`, `429`) even when NDJSON was asked for.

## Lines

Each line is one complete JSON object followed by `\n`. Split on `\n` only. Item lines are exactly the objects the JSON
form lists, in the same order, with the same keys (sorted). Text that came from the program under test is under `guest`
in an item line, as in the JSON form; it is JSON-escaped, so it cannot contain a line break or start a line of its own.

The **last line is always the control line**, which has the key `_next`:

| Control line | Meaning |
|---|---|
| `{"_next":"<cursor>"}` | There are more items. Pass the cursor as `?cursor=` to get the next page. |
| `{"_next":null}` | The list is complete. |

A body that does not end in a control line was cut short and should be treated as incomplete. No item line has an
`_next` key. Lists with no cursor (runs, failures, workers, targets, coverage) always end `{"_next":null}`; they are
bounded by `limit` (at most 500) like the JSON form.

Fields of the JSON form that describe the page rather than an item (a property table's `claim`, `notes`, `headline`,
`counts_by_status`, `total`, `guest_note`; a trial page's `run_id` and `scanned_trials`; a coverage report's `trials` and
`downsampled`) are kept, under `page` on the control line, so that labels such as "unverified worker data" are not lost:

```
$ curl -s -H 'Authorization: Bearer $TOKEN' -H 'Accept: application/x-ndjson' \
    'http://localhost:8080/api/runs/run_0123456789abcdef/trials?limit=2'
{"class":"...","guest":{...},"seed":"10","seq":0,"verdict":"pass", ...}
{"class":"...","guest":{...},"seed":"11","seq":1,"verdict":"violation", ...}
{"_next":"2","page":{"guest_note":"...","run_id":"run_0123456789abcdef","scanned_trials":2}}
```

The next call is the same URL with `&cursor=2`; the last page ends with `{"_next":null,...}`.

## From the command line

`--json` prints the same lines to stdout and nothing else (errors go to stderr, with exit code 2).

`hairspring results --json [--limit N]` prints one line per run, newest first (the same objects as `GET /api/runs`),
then `{"_next":null}`. With no runs, only the control line is printed.

`hairspring show <run> --json` prints:

1. One line with `"_kind":"run"`: the run (`run`), its `failures`, its property summary (`properties`, with
   `unverified_worker_data`, `ingest_incomplete` and `stopped_at_first_failure`), and the property table's page-level
   fields under `page`.
2. One line per property with `"_kind":"property"`, in table order (the same objects as the REST `properties` items).
3. `{"_next":null}`. The CLI follows every page, so the list is always complete.

```
{"_kind":"run","failures":[],"page":{"claim":{...},"total":1,...},"properties":{...},"run":{"id":"run_...","seed":1,...}}
{"_kind":"property","evals":4,"first_failing_seed":"2","guest":{...},"idx":0,"status":"violation",...}
{"_next":null}
```

Keys are in sorted order in every line, so output is byte-stable for the same data.
