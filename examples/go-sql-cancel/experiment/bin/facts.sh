#!/usr/bin/env bash
# facts.sh : sourced by the N and G drivers (byte-identical in both specs). Reads ONE run's stdout and stderr files and sets the F_* variables, in pure bash
# (no fork per line, so a native run of tens of milliseconds is not dominated by parsing). It records FACTS only; classes are assigned by sclib.py (classify).
# Order of facts_tsv = sclib.FACT_COLS. A fact that is absent is -1 (counters of a report) or 0 (counts of lines).
# shellcheck shell=bash
RE_STDOUT='^sqlcancel mode=([a-z]+) queries= *([0-9]+) scan_ok= *([0-9]+) scan_err= *([0-9]+) foreign_head= *([0-9]+) torn= *([0-9]+) unparsable= *([0-9]+) overlapped= *([0-9]+) foreign_next= *([0-9]+)$'
RE_ASSERT='^HAIRSPRING_EVENT \{"v":1,"kind":"assert","proc":"[^"]*","id":"([^"]+)","hits": *([0-9]+),"trues": *([0-9]+),"falses": *([0-9]+)'
RE_END='^HAIRSPRING_EVENT \{"v":1,"kind":"end","proc":"[^"]*","reason":"[^"]*","status": *(-?[0-9]+)'
RE_VIOL='^HAIRSPRING_VIOLATION: ([A-Za-z0-9._:/-]+):'
RE_GOFATAL='^(fatal error:|panic:|SIGSEGV|unexpected signal|runtime: )'
RE_HINFRA='HERMIT_INTERNAL_FAILURE|HERMIT_CLEANUP_UNCONFIRMED|tool_global\.rs:1619|SKID_OVERSHOOT|Usage: hermit'

facts_stderr_line() { local line=$1 id
  case $line in
    "HAIRSPRING_EVENT "*)
      F_NEV=$((F_NEV + 1))
      if [[ $line =~ $RE_ASSERT ]]; then
        id=${BASH_REMATCH[1]}
        case $id in
          go.sql.scan_returns_own_row) F_PT=${BASH_REMATCH[3]}; F_PF=${BASH_REMATCH[4]};;
          go.sql.harness.*) [ "$F_HF" = -1 ] && F_HF=0; F_HF=$((F_HF + BASH_REMATCH[4]));;
          go.sql.cancel_overlapped_scan) F_OVLT=${BASH_REMATCH[3]};;
          go.sql.scan_error_after_cancel) F_ERRH=${BASH_REMATCH[2]};;
          go.sql.conn_handed_to_waiter) F_HANDT=${BASH_REMATCH[3]};;
          *) ;;
        esac
      elif [[ $line =~ $RE_END ]]; then
        F_END=${BASH_REMATCH[1]}
      fi;;
    "HAIRSPRING_VIOLATION"*)
      F_NVIOL=$((F_NVIOL + 1))
      if [[ $line =~ $RE_VIOL ]]; then
        case ${BASH_REMATCH[1]} in
          go.sql.scan_returns_own_row) F_VPROP=$((F_VPROP + 1));;
          go.sql.harness.*) F_VHARN=$((F_VHARN + 1));;
          *) F_VOTHER=$((F_VOTHER + 1));;
        esac
      else
        F_VOTHER=$((F_VOTHER + 1))
      fi;;
    *) [ -n "$line" ] && F_NOTHER=$((F_NOTHER + 1));;
  esac
  [[ $line =~ $RE_GOFATAL ]] && F_FATAL=1
  [[ $line == *"panicked at"* ]] && F_HPANIC=1
  [[ $line == *"Deadlock detected"* ]] && F_DEAD=1
  [[ $line =~ $RE_HINFRA ]] && F_HINFRA=1
  return 0; }

# facts_row STDOUT_FILE STDERR_FILE
facts_row() { local so=$1 se=$2 line
  F_Q=-1 F_SCAN_OK=-1 F_SCAN_ERR=-1 F_FH=-1 F_TORN=-1 F_UNP=-1 F_OVL=-1 F_FNEXT=-1 F_NOUT_OTHER=0
  F_NEV=0 F_NVIOL=0 F_NOTHER=0 F_PF=-1 F_PT=-1 F_HF=-1 F_OVLT=-1 F_ERRH=-1 F_HANDT=-1 F_END=-1
  F_VPROP=0 F_VHARN=0 F_VOTHER=0 F_FATAL=0 F_HPANIC=0 F_HINFRA=0 F_DEAD=0
  if [ -s "$so" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
      if [[ $line =~ $RE_STDOUT ]]; then
        F_Q=${BASH_REMATCH[2]} F_SCAN_OK=${BASH_REMATCH[3]} F_SCAN_ERR=${BASH_REMATCH[4]} F_FH=${BASH_REMATCH[5]}
        F_TORN=${BASH_REMATCH[6]} F_UNP=${BASH_REMATCH[7]} F_OVL=${BASH_REMATCH[8]} F_FNEXT=${BASH_REMATCH[9]}
      elif [ -n "$line" ]; then
        F_NOUT_OTHER=$((F_NOUT_OTHER + 1))
      fi
      [[ $line =~ $RE_GOFATAL ]] && F_FATAL=1
    done < "$so"
  fi
  if [ -s "$se" ]; then
    while IFS= read -r line || [ -n "$line" ]; do facts_stderr_line "$line"; done < "$se"
  fi
  return 0; }

facts_tsv() { printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "$F_Q" "$F_SCAN_OK" "$F_SCAN_ERR" "$F_FH" "$F_TORN" "$F_UNP" "$F_OVL" "$F_FNEXT" "$F_NOUT_OTHER" "$F_NEV" "$F_NVIOL" "$F_NOTHER" \
  "$F_PF" "$F_PT" "$F_HF" "$F_OVLT" "$F_ERRH" "$F_HANDT" "$F_END" "$F_VPROP" "$F_VHARN" "$F_VOTHER" "$F_FATAL" "$F_HPANIC" "$F_HINFRA" "$F_DEAD"; }
