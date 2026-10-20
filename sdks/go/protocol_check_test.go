package hairspring

// A strict checker for guest event protocol v1, written from docs/guest-protocol.md and tested against the shared
// goldens in sdks/conformance (see conformance_test.go). It is test code on purpose: the SDK's own line builders
// are checked against an independent reading of the rules.

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"regexp"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

const (
	checkMaxLine   = 4096
	checkMaxDepth  = 4
	checkEvalProc  = "_eval"
	checkPrefix    = "HAIRSPRING_EVENT"
	checkMaxThread = 256
)

var (
	checkIdent  = regexp.MustCompile(`^[A-Za-z0-9._:/-]+$`)
	checkThread = regexp.MustCompile(`^[A-Za-z0-9._:/#-]+$`)
	checkUint   = regexp.MustCompile(`^(0|[1-9][0-9]*)$`)
	checkInt    = regexp.MustCompile(`^(0|-?[1-9][0-9]*)$`)
	checkKinds  = map[string]bool{"always": true, "always_or_unreachable": false, "sometimes": true, "reachable": true, "unreachable": false}
)

// checkedLine is a line that passed the checker, with its decoded object.
type checkedLine struct {
	kind string
	obj  map[string]any
}

// checkLine validates one line (without its newline) and returns its decoded form. A line with a version above 1 or
// a kind this checker does not know is accepted and returned with an empty kind, as the protocol says to ignore it.
func checkLine(line string) (checkedLine, error) {
	if len(line)+1 > checkMaxLine {
		return checkedLine{}, fmt.Errorf("line is %d bytes with its newline, cap is %d", len(line)+1, checkMaxLine)
	}
	if !utf8.ValidString(line) {
		return checkedLine{}, errors.New("not valid UTF-8")
	}
	body, ok := strings.CutPrefix(strings.TrimSuffix(line, "\r"), checkPrefix+" ")
	if !ok {
		return checkedLine{}, errors.New("missing the HAIRSPRING_EVENT prefix and its space")
	}
	body = strings.TrimLeft(body, " ")
	if err := checkJSONShape(body); err != nil {
		return checkedLine{}, err
	}
	dec := json.NewDecoder(strings.NewReader(body))
	dec.UseNumber()
	var obj map[string]any
	if err := dec.Decode(&obj); err != nil {
		return checkedLine{}, fmt.Errorf("not a JSON object: %w", err)
	}
	return checkObject(obj)
}

// checkJSONShape rejects trailing text, duplicate keys and nesting deeper than checkMaxDepth.
func checkJSONShape(body string) error {
	dec := json.NewDecoder(strings.NewReader(body))
	dec.UseNumber()
	first, err := dec.Token()
	if err != nil {
		return fmt.Errorf("not JSON: %w", err)
	}
	if first != json.Delim('{') {
		return errors.New("not a JSON object")
	}
	if err := walkContainer(dec, '{', 1); err != nil {
		return err
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return errors.New("trailing text after the object")
	}
	return nil
}

func walkContainer(dec *json.Decoder, open json.Delim, depth int) error {
	if depth > checkMaxDepth {
		return fmt.Errorf("nesting deeper than %d", checkMaxDepth)
	}
	seen := map[string]bool{}
	for expectKey := open == '{'; dec.More(); {
		if expectKey {
			tok, err := dec.Token()
			if err != nil {
				return err
			}
			key, _ := tok.(string)
			if seen[key] {
				return fmt.Errorf("duplicate key %q", key)
			}
			seen[key] = true
		}
		if err := walkValue(dec, depth); err != nil {
			return err
		}
	}
	_, err := dec.Token()
	return err
}

func walkValue(dec *json.Decoder, depth int) error {
	tok, err := dec.Token()
	if err != nil {
		return err
	}
	if d, ok := tok.(json.Delim); ok && (d == '{' || d == '[') {
		return walkContainer(dec, d, depth+1)
	}
	return nil
}

func checkObject(obj map[string]any) (checkedLine, error) {
	v, err := fieldUint(obj, "v")
	if err != nil {
		return checkedLine{}, err
	}
	if v == 0 {
		return checkedLine{}, errors.New("v must be at least 1")
	}
	kind, err := fieldString(obj, "kind", 64)
	if err != nil {
		return checkedLine{}, err
	}
	if v > 1 {
		return checkedLine{}, nil
	}
	checker, known := kindCheckers[kind]
	if !known {
		return checkedLine{}, nil
	}
	if err := checker(obj); err != nil {
		return checkedLine{}, fmt.Errorf("%s line: %w", kind, err)
	}
	return checkedLine{kind: kind, obj: obj}, nil
}

var kindCheckers = map[string]func(map[string]any) error{
	"hello": checkHello, "catalog": checkCatalog, "assert": checkAssert, "work": checkWork,
	"thread": checkThreadLine, "end": checkEnd, "eval": checkEval,
}

func checkHello(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	for _, f := range []string{"sdk", "sdk_version", "catalog_hash"} {
		if _, err := fieldText(o, f, 128, true); err != nil {
			return err
		}
	}
	if _, err := fieldBool(o, "signal_handlers", true); err != nil {
		return err
	}
	_, err := fieldBool(o, "tag_stable", false)
	return err
}

func checkCatalog(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	if _, err := fieldIdent(o, "id", 128); err != nil {
		return err
	}
	typ, err := fieldString(o, "type", 64)
	if err != nil {
		return err
	}
	mustHit, known := checkKinds[typ]
	if !known {
		return fmt.Errorf("unknown type %q", typ)
	}
	got, err := fieldBool(o, "must_hit", true)
	if err != nil {
		return err
	}
	if got != mustHit {
		return fmt.Errorf("must_hit=%t disagrees with type %q", got, typ)
	}
	if _, err := fieldText(o, "msg", 512, false); err != nil {
		return err
	}
	return checkLoc(o)
}

func checkLoc(o map[string]any) error {
	raw, present := o["loc"]
	if !present {
		return nil
	}
	loc, ok := raw.(map[string]any)
	if !ok {
		return errors.New("loc is not an object")
	}
	if _, err := fieldText(loc, "file", 256, true); err != nil {
		return err
	}
	if _, err := fieldUint(loc, "line"); err != nil {
		return err
	}
	_, err := fieldText(loc, "func", 128, false)
	return err
}

func checkAssert(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	if _, err := fieldIdent(o, "id", 128); err != nil {
		return err
	}
	counts := map[string]uint64{}
	for _, f := range []string{"hits", "trues", "falses"} {
		n, err := fieldUint(o, f)
		if err != nil {
			return err
		}
		counts[f] = n
	}
	if counts["hits"] != counts["trues"]+counts["falses"] {
		return fmt.Errorf("hits %d is not trues %d plus falses %d", counts["hits"], counts["trues"], counts["falses"])
	}
	if err := checkWitness(o, "first_false", counts["falses"]); err != nil {
		return err
	}
	return checkWitness(o, "first_true", counts["trues"])
}

func checkWitness(o map[string]any, key string, count uint64) error {
	raw, present := o[key]
	if !present || raw == nil {
		return nil
	}
	w, ok := raw.(map[string]any)
	if !ok {
		return fmt.Errorf("%s is neither null nor an object", key)
	}
	if count == 0 {
		return fmt.Errorf("%s is set but the count behind it is 0", key)
	}
	if _, err := fieldThread(w); err != nil {
		return fmt.Errorf("%s: %w", key, err)
	}
	ord, err := fieldUint(w, "ord")
	if err != nil || ord == 0 {
		return fmt.Errorf("%s: ord must be an unsigned integer from 1", key)
	}
	vals, ok := w["vals"].([]any)
	if !ok || len(vals) != 4 {
		return fmt.Errorf("%s: vals must be exactly four integers", key)
	}
	for _, x := range vals {
		if err := intNumber(x); err != nil {
			return fmt.Errorf("%s: vals: %w", key, err)
		}
	}
	return nil
}

func checkWork(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	_, err := fieldUint(o, "ops")
	return err
}

func checkThreadLine(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	if _, err := fieldThread(o); err != nil {
		return err
	}
	for _, f := range []string{"evals", "tape_consumed"} {
		if _, err := fieldUint(o, f); err != nil {
			return err
		}
	}
	_, err := fieldBool(o, "key_stable", false)
	return err
}

func checkEnd(o map[string]any) error {
	if err := fieldProc(o); err != nil {
		return err
	}
	reason, err := fieldString(o, "reason", 64)
	if err != nil {
		return err
	}
	if reason != "exit" && reason != "signal" && reason != "explicit" {
		return fmt.Errorf("unknown reason %q", reason)
	}
	if err := intNumber(o["status"]); err != nil {
		return fmt.Errorf("status: %w", err)
	}
	for _, f := range []string{"sites", "evals", "dropped"} {
		if _, err := fieldUint(o, f); err != nil {
			return err
		}
	}
	threads, err := fieldUint(o, "threads")
	if err != nil {
		return err
	}
	if threads > checkMaxThread {
		return fmt.Errorf("threads %d is over %d", threads, checkMaxThread)
	}
	return nil
}

func checkEval(o map[string]any) error {
	if _, err := fieldIdent(o, "id", 128); err != nil {
		return err
	}
	if typ, err := fieldString(o, "type", 64); err != nil {
		return err
	} else if _, known := checkKinds[typ]; !known {
		return fmt.Errorf("unknown type %q", typ)
	}
	if _, err := fieldBool(o, "ok", true); err != nil {
		return err
	}
	if raw, present := o["proc"]; present && raw != checkEvalProc {
		return fmt.Errorf("an eval line belongs to %q, not %v", checkEvalProc, raw)
	}
	return nil
}

func fieldString(o map[string]any, key string, max int) (string, error) {
	s, ok := o[key].(string)
	if !ok || s == "" || len(s) > max {
		return "", fmt.Errorf("%s must be a string of 1 to %d bytes", key, max)
	}
	return s, nil
}

func fieldIdent(o map[string]any, key string, max int) (string, error) {
	s, err := fieldString(o, key, max)
	if err == nil && !checkIdent.MatchString(s) {
		err = fmt.Errorf("%s %q has characters outside [A-Za-z0-9._:/-]", key, s)
	}
	return s, err
}

func fieldProc(o map[string]any) error {
	s, err := fieldIdent(o, "proc", 64)
	if err == nil && s == checkEvalProc {
		err = fmt.Errorf("proc %q is reserved for eval lines", checkEvalProc)
	}
	return err
}

func fieldThread(o map[string]any) (string, error) {
	s, err := fieldString(o, "thread", 64)
	if err == nil && !checkThread.MatchString(s) {
		err = fmt.Errorf("thread key %q has characters outside [A-Za-z0-9._:/#-]", s)
	}
	return s, err
}

// fieldText is a free-text field: bytes limited, no control characters and no Unicode format or separator characters.
func fieldText(o map[string]any, key string, max int, required bool) (string, error) {
	raw, present := o[key]
	if !present {
		if required {
			return "", fmt.Errorf("%s is missing", key)
		}
		return "", nil
	}
	s, ok := raw.(string)
	if !ok || len(s) > max {
		return "", fmt.Errorf("%s must be a string of at most %d bytes", key, max)
	}
	for _, r := range s {
		if unicode.IsControl(r) || unicode.In(r, unicode.Cf, unicode.Zl, unicode.Zp) {
			return "", fmt.Errorf("%s has the forbidden character U+%04X", key, r)
		}
	}
	return s, nil
}

func fieldBool(o map[string]any, key string, required bool) (bool, error) {
	raw, present := o[key]
	if !present && !required {
		return false, nil
	}
	b, ok := raw.(bool)
	if !ok {
		return false, fmt.Errorf("%s must be a boolean", key)
	}
	return b, nil
}

func fieldUint(o map[string]any, key string) (uint64, error) {
	n, ok := o[key].(json.Number)
	if !ok || !checkUint.MatchString(n.String()) {
		return 0, fmt.Errorf("%s must be an unsigned integer (no sign, fraction or exponent)", key)
	}
	u, err := strconv.ParseUint(n.String(), 10, 64)
	if err != nil {
		return 0, fmt.Errorf("%s is out of range: %w", key, err)
	}
	return u, nil
}

func intNumber(raw any) error {
	n, ok := raw.(json.Number)
	if !ok || !checkInt.MatchString(n.String()) {
		return errors.New("not an integer")
	}
	if _, err := strconv.ParseInt(n.String(), 10, 64); err != nil {
		return errors.New("out of the int64 range")
	}
	return nil
}

// checkTrial validates a whole report written by one process: each line, then the cross-checks of the protocol.
func checkTrial(lines []string) error {
	var end checkedLine
	catalog := map[string]string{} // id -> type
	asserts := map[string]checkedLine{}
	var threads, helloCount, endCount int
	var evals uint64
	var proc string
	for i, l := range lines {
		cl, err := checkLine(strings.TrimSuffix(l, "\n"))
		if err != nil {
			return fmt.Errorf("line %d: %w", i+1, err)
		}
		if p, ok := cl.obj["proc"].(string); ok {
			if proc != "" && p != proc {
				return fmt.Errorf("line %d: proc %q, earlier lines used %q", i+1, p, proc)
			}
			proc = p
		}
		switch cl.kind {
		case "hello":
			helloCount++
			if i != 0 {
				return fmt.Errorf("line %d: hello is not the first line", i+1)
			}
		case "catalog":
			id := cl.obj["id"].(string)
			if _, dup := catalog[id]; dup {
				return fmt.Errorf("line %d: id %q catalogued twice (sites must be merged)", i+1, id)
			}
			catalog[id] = cl.obj["type"].(string)
		case "assert":
			id := cl.obj["id"].(string)
			if _, dup := asserts[id]; dup {
				return fmt.Errorf("line %d: two assert lines for %q", i+1, id)
			}
			asserts[id] = cl
			n, _ := fieldUint(cl.obj, "hits")
			evals += n
		case "thread":
			threads++
		case "end":
			endCount++
			end = cl
			if i != len(lines)-1 {
				return fmt.Errorf("line %d: end is not the last line", i+1)
			}
		}
	}
	if helloCount != 1 || endCount != 1 {
		return fmt.Errorf("want one hello and one end, got %d and %d", helloCount, endCount)
	}
	return crossCheck(catalog, asserts, end.obj, threads, evals)
}

func crossCheck(catalog map[string]string, asserts map[string]checkedLine, end map[string]any, threads int, evals uint64) error {
	for id, typ := range catalog {
		a, ok := asserts[id]
		if !ok {
			return fmt.Errorf("%q is catalogued but has no assert line", id)
		}
		if typ == "reachable" || typ == "unreachable" {
			if witness := a.obj["first_false"]; witness != nil {
				return fmt.Errorf("%q is a %s site and must not send first_false", id, typ)
			}
			if falses, _ := fieldUint(a.obj, "falses"); falses != 0 {
				return fmt.Errorf("%q is a %s site with falses=%d", id, typ, falses)
			}
		}
	}
	for id := range asserts {
		if _, ok := catalog[id]; !ok {
			return fmt.Errorf("%q has an assert line but no catalog line", id)
		}
	}
	if sites, _ := fieldUint(end, "sites"); sites != uint64(len(catalog)) {
		return fmt.Errorf("end.sites=%d but %d ids were catalogued", sites, len(catalog))
	}
	if n, _ := fieldUint(end, "threads"); n != uint64(threads) {
		return fmt.Errorf("end.threads=%d but %d thread lines arrived", n, threads)
	}
	if n, _ := fieldUint(end, "evals"); n != evals {
		return fmt.Errorf("end.evals=%d but the assert lines add up to %d", n, evals)
	}
	return nil
}

// splitLines splits output into lines without their newlines and reports a final line that lacks one.
func splitLines(out []byte) ([]string, error) {
	if len(out) == 0 {
		return nil, nil
	}
	if !bytes.HasSuffix(out, []byte("\n")) {
		return nil, errors.New("output does not end with a newline")
	}
	return strings.Split(strings.TrimSuffix(string(out), "\n"), "\n"), nil
}
