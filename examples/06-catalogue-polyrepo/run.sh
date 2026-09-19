#!/usr/bin/env bash
# 06-catalogue-polyrepo — one model, four repositories.
#
# The model is composed from chunks that live in four separate repositories:
#
#   repos/catalogue  the shared table of physical containers, authored once as
#                    data, plus the two bindings over it — one derived from the
#                    site, one left free — and the services' inheritance root
#   repos/vision     a service that requires {container: line_container}
#   repos/compute    a second service that requires the SAME binding, and
#                    additionally lists the entries it accepts
#   repos/sorter     a third service that requires a DIFFERENT binding over the
#                    same table, and brings a catalogue and a binding of its own
#
# Steps (each one self-checks; the script exits non-zero on any surprise):
#   1. Compile the four chunks — and, in passing, the two properties that make
#      multi-repository work practical: relocating a repository leaves the
#      model_hash alone (ADR-0056), and the compile set is the unit of
#      verification (a closed subset compiles, a lone repository does not).
#   2. Resolve both sites for all three service scopes. Each service reads its
#      OWN requires block: vision and compute share the line's container, the
#      sorter gets a different one plus its own lane profile.
#   3. `cfx options` offers only the container the selected site is equipped for.
#   4. Three forced conflicts, refused by name: the derive table, the accepts
#      list, and a facet-equality constraint between the two bindings.
#   5. Binding one facet twice with different options is a usage error.
#   6. Resolution is deterministic: the same selection twice, byte for byte.
#   7. The same model built one repository at a time: an object per unit, a
#      link that reproduces step 1's package byte for byte and pins what it
#      linked, and the two mistakes between units that only a link can name.
#
# Drift-checking: this example's committed JSON is guarded by
# //examples:export_pack_test (a fresh examples/export_pack.sh run must be
# byte-identical), NOT by compiler/cue/export_fixtures.sh, whose sweep covers
# only the two-file and single-file layouts under examples/*/cue.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
OUT_DIR="${CONFIGFLUX_EXAMPLE_OUT_DIR:-${EXAMPLE_DIR}/out}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
find_binary() {
  local env_var="$1" bazel_rel="$2" pretty="$3"
  local override="${!env_var:-}"
  if [[ -n "${override}" ]]; then
    echo "${override}"
    return
  fi
  local bazel_bin="${REPO_ROOT}/${bazel_rel}"
  if [[ -x "${bazel_bin}" ]]; then
    echo "${bazel_bin}"
    return
  fi
  echo >&2 "Error: ${pretty} binary not found."
  echo >&2 "  Either build it:  bazel build //compiler //cfx"
  echo >&2 "  Or set:           export ${env_var}=/path/to/${pretty}"
  exit 1
}

banner() { printf "\n=== %s ===\n" "$1"; }
ok()     { printf "  -> %s\n" "$1"; }

COMPILER="$(find_binary CONFIGFLUX_COMPILER bazel-bin/compiler/compiler compiler)"
CFX="$(find_binary CONFIGFLUX_CFX bazel-bin/cfx/cfx cfx)"

CATALOGUE="${EXAMPLE_DIR}/repos/catalogue/00_catalogue.json"
VISION="${EXAMPLE_DIR}/repos/vision/10_vision.json"
COMPUTE="${EXAMPLE_DIR}/repos/compute/10_compute.json"
SORTER="${EXAMPLE_DIR}/repos/sorter/20_sorter.json"
MANIFEST="${EXAMPLE_DIR}/environments.json"

# Read the compile result's status / model_hash without a jq dependency.
compile_field() {
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2],""))' "$1" "$2"
}

# One field of one requirement slot, read the way a SERVICE reads it: find the
# service's own block, then the field. No catalogue id, no other component's
# parameter path — this is the whole point of the requires block.
requirement_field() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
out=d["resolved_output"]
root=next(iter(out))
service=out[root]["components"][sys.argv[2]]
print(service["requires"][sys.argv[3]]["fields"][sys.argv[4]])' "$1" "$2" "$3" "$4"
}

# The catalogue entry a service was given, by id.
requirement_entry() {
  python3 -c 'import json,sys
d=json.load(open(sys.argv[1]))
out=d["resolved_output"]
root=next(iter(out))
print(out[root]["components"][sys.argv[2]]["requires"][sys.argv[3]]["entry"])' "$1" "$2" "$3"
}

# One service's whole requires block, as the service sees it.
print_requires_block() {
  python3 -c 'import json,sys
out=json.load(open(sys.argv[1]))["resolved_output"]
root=next(iter(out))
print("     "+json.dumps(out[root]["components"][sys.argv[2]]["requires"],sort_keys=True))' "$1" "$2"
}

snapshot_path() {
  local dir="$1"
  local -a found
  shopt -s nullglob
  found=("${dir}"/resolve_result.*.json)
  shopt -u nullglob
  [[ ${#found[@]} -eq 1 ]] \
    || { echo "expected 1 resolve_result.*.json under ${dir}, found ${#found[@]}" >&2; exit 1; }
  printf '%s' "${found[0]}"
}

expect_equal() {
  local what="$1" want="$2" got="$3"
  [[ "${want}" == "${got}" ]] \
    || { echo "${what}: expected '${want}', got '${got}'" >&2; exit 1; }
}

expect_grep() {
  local what="$1" needle="$2" file="$3"
  grep -q -- "${needle}" "${file}" \
    || { echo "${what}: '${needle}' not found in ${file}" >&2; exit 1; }
}

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

# Compile an arbitrary set of chunks into ${OUT_DIR}/<tag>; echo the model path.
compile_set() {
  local tag="$1"; shift
  local -a sources=()
  local src
  for src in "$@"; do sources+=(--source "${src}"); done
  "${COMPILER}" compile "${sources[@]}" --out "${OUT_DIR}/${tag}" \
    > "${OUT_DIR}/compile_${tag}.json"
  expect_equal "${tag} compile status" "ok" \
    "$(compile_field "${OUT_DIR}/compile_${tag}.json" status)"
  printf '%s' "${OUT_DIR}/${tag}/cmp.manifest.json"
}

# ---------------------------------------------------------------------------
# Step 1: compile four chunks from four directories
# ---------------------------------------------------------------------------
banner "Step 1: Compile the four repositories into one model"
MODEL="$(compile_set cmp "${CATALOGUE}" "${VISION}" "${COMPUTE}" "${SORTER}")"
[[ -f "${MODEL}" ]] || { echo "compile: cmp.manifest.json missing" >&2; exit 1; }
BASE_HASH="$(compile_field "${OUT_DIR}/compile_cmp.json" model_hash)"
ok "compiled 4 chunks from 4 directories -> ${MODEL}"
ok "model_hash ${BASE_HASH}"

# The property that makes multi-repository composition workable in practice. A
# job checks one repository out somewhere else — a scratch directory, a vendored
# copy, another checkout — and compiles it with the other repositories' exported
# chunks. If identity depended on where the files sat, every such checkout would
# produce a different model and no hash could be compared across machines.
assert_model_hash_is_path_invariant() {
  local worktree="${OUT_DIR}/vision-worktree"
  rm -rf "${worktree}"
  mkdir -p "${worktree}"
  cp "${VISION}" "${worktree}/10_vision.json"
  local relocated
  relocated="$(compile_set cmp-worktree \
    "${CATALOGUE}" "${worktree}/10_vision.json" "${COMPUTE}" "${SORTER}")"
  expect_equal "model_hash across a relocated repository" "${BASE_HASH}" \
    "$(compile_field "${OUT_DIR}/compile_cmp-worktree.json" model_hash)"
  ok "vision repository re-checked-out at ${worktree}; model_hash unchanged"
  [[ -f "${relocated}" ]] || { echo "relocated compile wrote no model" >&2; exit 1; }
}
assert_model_hash_is_path_invariant

# The compile set is the unit of verification. A dependency-closed SUBSET is a
# legitimate model — catalogue + vision, with its own hash, because it contains
# different components. A repository whose requirement names a binding the
# compile set has never seen is not, and that is the point.
compile_set cmp-subset "${CATALOGUE}" "${VISION}" > /dev/null
SUBSET_HASH="$(compile_field "${OUT_DIR}/compile_cmp-subset.json" model_hash)"
[[ "${SUBSET_HASH}" != "${BASE_HASH}" ]] \
  || { echo "subset: expected a different model_hash than the full set" >&2; exit 1; }
ok "catalogue + vision is a legitimate closed subset (model_hash ${SUBSET_HASH})"

ALONE_OUT="${OUT_DIR}/compile_alone.json"
alone_rc=0
"${COMPILER}" compile --source "${VISION}" --out "${OUT_DIR}/cmp-alone" \
  > "${ALONE_OUT}" 2>/dev/null || alone_rc=$?
[[ "${alone_rc}" -ne 0 ]] \
  || { echo "compile: expected a non-zero exit for the un-closed set" >&2; exit 1; }
expect_equal "vision-alone compile status" "error" "$(compile_field "${ALONE_OUT}" status)"
expect_grep "vision alone" "E_REQUIRES_INVALID" "${ALONE_OUT}"
ok "vision alone is refused (E_REQUIRES_INVALID): its binding lives elsewhere"

# ---------------------------------------------------------------------------
# Step 2: resolve both sites, for all three services
# ---------------------------------------------------------------------------
# environments.json states, per site, ONLY the decisions the model does not make
# for itself: the site, the sorter's container, and its lane profile.
# `line_container` is never stated — the derive table decides it from the site.
#
# Each snapshot carries the catalogue entry INSIDE the service that required it.
# A requirement is not a depends_on edge, so the catalogue is not dragged into
# the service's dependency closure — and the service still gets its entry.
#
# REQ-CFX-014: the requires block is DELIVERED, and sameness is by binding —
# two services naming one binding get one entry; a third naming another binding
# over the same table gets its own.
assert_requires_delivered_per_service() {
  local site="$1" line_entry="$2" line_dims="$3" lanes_entry="$4" lane_fields="$5"
  local service snap entry dims dim
  for service in vision_service compute_service; do
    snap="$(snapshot_path "${RESOLVED}/${site}/${service}")"
    entry="$(requirement_entry "${snap}" "${service}" container)"
    expect_equal "${site}/${service} container entry" "${line_entry}" "${entry}"
    dims=""
    for dim in length_mm width_mm height_mm; do
      dims="${dims}${dims:+ }$(requirement_field "${snap}" "${service}" container "${dim}")"
    done
    expect_equal "${site}/${service} container dimensions" "${line_dims}" "${dims}"
    ok "${site} / ${service}: requires.container = ${entry} -> ${dims} mm (l w h)"
    print_requires_block "${snap}" "${service}"
  done

  snap="$(snapshot_path "${RESOLVED}/${site}/sorter_service")"
  entry="$(requirement_entry "${snap}" sorter_service container)"
  expect_equal "${site}/sorter_service container entry" "c3" "${entry}"
  [[ "${entry}" != "${line_entry}" ]] \
    || { echo "${site}: the sorter must NOT share the line's container" >&2; exit 1; }
  expect_equal "${site}/sorter_service lane profile" "${lanes_entry}" \
    "$(requirement_entry "${snap}" sorter_service lanes)"
  dims=""
  for dim in belt_width_mm lane_count; do
    dims="${dims}${dims:+ }$(requirement_field "${snap}" sorter_service lanes "${dim}")"
  done
  expect_equal "${site}/sorter_service lane fields" "${lane_fields}" "${dims}"
  ok "${site} / sorter_service: container = ${entry} (its own), lanes = ${lanes_entry} -> ${dims}"
  print_requires_block "${snap}" sorter_service
}

banner "Step 2: Resolve every site for all three services"
RESOLVED="${OUT_DIR}/resolved"
"${CFX}" resolve \
  --model "${MODEL}" \
  --manifest "${MANIFEST}" \
  --all \
  --scopes component:vision_service,component:compute_service,component:sorter_service \
  --out "${RESOLVED}" \
  > "${OUT_DIR}/resolve.log"

assert_requires_delivered_per_service factory_a c1 "1200 800 1000" narrow "400 2"
assert_requires_delivered_per_service factory_b c2 "800 600 700" wide "900 4"
ok "vision and compute share an entry by naming ONE binding; the sorter names another"
ok "no service names the catalogue, and none names another service"

# ---------------------------------------------------------------------------
# Step 3: the wrong container is never offered in the first place
# ---------------------------------------------------------------------------
banner "Step 3: cfx options — factory_b is only offered its own container"
OPTS="${OUT_DIR}/options.factory_b.json"
"${CFX}" options --model "${MODEL}" --select site=factory_b --format json > "${OPTS}"
python3 -c 'import json,sys
rows=json.load(open(sys.argv[1]))
row=[r for r in rows if r.get("facet")=="line_container"]
assert row, "line_container binding missing from cfx options output"
opts=row[0].get("valid_options", [])
assert opts==["c2"], f"expected line_container=[c2] at factory_b, got {opts}"
print("  -> line_container valid options at factory_b:", opts)' "${OPTS}"

# ---------------------------------------------------------------------------
# Step 4: three forced conflicts, each refused BY NAME
# ---------------------------------------------------------------------------
# `cfx resolve` exits 3 and writes nothing; `cfx explain` then names the rule —
# or, through a derived binding, every rule on the chain (step 4c).
# The two exit codes run opposite ways on purpose: explain exits 0 when it has
# an explanation to give, and 3 when the selection turned out to be fine.
#
# The attribution id is read out of `cfx explain --format json`, which carries it
# verbatim; the text rendering decodes it back into the construct the author
# wrote, so it is the JSON that pins the rule's identity.
#
# The attribution argument is ONE id, or SEVERAL comma-separated: a contradiction
# reached THROUGH a derived binding puts every link of the chain in the core, and
# the case is only honestly pinned if all of them are named (step 4c).
assert_refused_by() {
  local label="$1" attribution="$2"; shift 2
  local -a attributions=(); IFS=',' read -r -a attributions <<< "${attribution}"
  local -a selects=()
  local model="${MODEL}" s a
  if [[ "$1" == "--model" ]]; then model="$2"; shift 2; fi
  for s in "$@"; do selects+=(--select "${s}"); done

  local out="${OUT_DIR}/refused-${label}"
  local err="${OUT_DIR}/refused-${label}.stderr"
  local rc=0
  "${CFX}" resolve --model "${model}" "${selects[@]}" --out "${out}" --format text \
    > /dev/null 2> "${err}" || rc=$?
  expect_equal "${label}: resolve exit code" "3" "${rc}"
  expect_grep "${label}: resolve diagnostic" "E_SELECTION_CONFLICT" "${err}"
  [[ ! -e "${out}" ]] \
    || { echo "${label}: a rejected selection still wrote ${out}" >&2; exit 1; }

  local exp="${OUT_DIR}/explain-${label}.json"
  local exp_rc=0
  "${CFX}" explain --model "${model}" "${selects[@]}" --format json > "${exp}" || exp_rc=$?
  expect_equal "${label}: explain exit code" "0" "${exp_rc}"
  for a in "${attributions[@]}"; do
    expect_grep "${label}: explain attribution ${a}" "${a}" "${exp}"
  done
  "${CFX}" explain --model "${model}" "${selects[@]}" --format text | sed 's/^/     /'
  ok "${label}: refused (exit 3), explained as ${attributions[*]}, wrote nothing"
}

banner "Step 4a: the site's derive table refuses a container it did not pick"
assert_refused_by derive "derive:line_container:site=factory_a" \
  site=factory_a line_container=c2

banner "Step 4b: compute's accepts list refuses the entry it cannot handle"
assert_refused_by accepts "accepts:compute_service.container" \
  line_container=c3

# The facet-equality constraint ships COMMENTED OUT in
# repos/sorter/cue/20_sorter.cue: with it enabled neither environment resolves,
# which is its whole point but a useless default. The HUMAN path to enabling it
# is to uncomment the block and re-export the pack with the README's command.
# This script cannot take that path — the example test's runfiles contain no
# `cue` evaluator — so it does the machine-checkable EQUIVALENT: inject the same
# constraint object into a COPY of the already-exported chunk and compile that.
# The CUE is not re-exported; the committed JSON on disk is untouched.
banner "Step 4c: a facet-equality constraint ties the two bindings together"
PATCHED_SORTER="${OUT_DIR}/patched/20_sorter.json"
mkdir -p "${OUT_DIR}/patched"
python3 -c 'import json,sys
chunk=json.load(open(sys.argv[1]))
chunk["constraints"]={"sorter_matches_line":{
    "condition":"sorter_container == line_container",
    "doc":"The sorter and the line handle the same container."}}
json.dump(chunk,open(sys.argv[2],"w"),indent=4)' "${SORTER}" "${PATCHED_SORTER}"
PATCHED_MODEL="$(compile_set cmp-equal \
  "${CATALOGUE}" "${VISION}" "${COMPUTE}" "${PATCHED_SORTER}")"
ok "recompiled with sorter_matches_line enabled on a copy of the sorter chunk"
# The shape an ENVIRONMENT actually writes: the site, plus the one container
# still free. `line_container` is never typed — the site's derive table decides
# it (ADR-0057 D3) — so the contradiction is reached THROUGH that binding. Drop
# the equality constraint and this resolves; drop the derive table and the site
# constrains nothing — so both ids are in ANY core here. Compute's accepts list
# is in it too, left unasserted: explain returns ONE of several minimal cores.
assert_refused_by equality \
  "sorter_matches_line,derive:line_container:site=factory_a" \
  --model "${PATCHED_MODEL}" site=factory_a sorter_container=c3

# ---------------------------------------------------------------------------
# Step 5: one facet, bound twice, two different options. A usage error (exit 2),
# not a resolution one: the input never described a single selection, so there
# is nothing to resolve and nothing to explain.
# ---------------------------------------------------------------------------
banner "Step 5: binding one facet twice with different options is refused"
DOUBLE_ERR="${OUT_DIR}/resolve.double-binding.stderr"
double_rc=0
"${CFX}" resolve --model "${MODEL}" \
  --select sorter_container=c3 --select sorter_container=c1 \
  --out "${OUT_DIR}/resolved-double" \
  > /dev/null 2> "${DOUBLE_ERR}" || double_rc=$?
cat "${DOUBLE_ERR}"
expect_equal "double-binding exit code" "2" "${double_rc}"
expect_grep "double binding names the facet" "sorter_container" "${DOUBLE_ERR}"
expect_grep "double binding names the first option" "'c3'" "${DOUBLE_ERR}"
expect_grep "double binding names the second option" "'c1'" "${DOUBLE_ERR}"
ok "rejected (exit 2), naming sorter_container and both options it was given"

# ---------------------------------------------------------------------------
# Step 6: the same selection resolves to the same bytes.
# ---------------------------------------------------------------------------
banner "Step 6: Resolution is deterministic"
for run in one two; do
  "${CFX}" resolve --model "${MODEL}" \
    --select site=factory_b --select sorter_container=c3 --select sorter_lanes=wide \
    --out "${OUT_DIR}/determinism-${run}" > "${OUT_DIR}/determinism-${run}.log"
done
DET_ONE="$(snapshot_path "${OUT_DIR}/determinism-one")"
DET_TWO="$(snapshot_path "${OUT_DIR}/determinism-two")"
cmp -s "${DET_ONE}" "${DET_TWO}" \
  || { echo "determinism: two resolves of one selection differ" >&2; exit 1; }
ok "two resolves of the same selection are byte-identical"
expect_grep "the container was implied, not stated" "implied: line_container=c2" \
  "${OUT_DIR}/determinism-one.log"
ok "line_container was never selected: the site implied c2"

# ---------------------------------------------------------------------------
# Step 7: the same model, built one repository at a time
# ---------------------------------------------------------------------------
# Steps 1-6 handed all four chunks to one `compile`. That is the shortcut for a
# checkout that holds every repository at once. Four teams on four machines do
# not have that checkout, so each repository is built ALONE, as a UNIT — the
# chunks sharing one `package` value — into a content-addressed OBJECT, and a
# LINK step assembles the objects into the package. `compile` is that same
# linker fed by objects it built in memory, so the two forms are one code path
# and the package below is byte-identical to Step 1's.
banner "Step 7: compile each repository into an object, then link the objects"
OBJ="${OUT_DIR}/objects"
mkdir -p "${OBJ}"

# One field of an object's header. The header is the whole cross-unit contract:
# what the unit exports, what it still needs from elsewhere, its clauses, and
# the hash of every interface it was compiled against.
object_field() {
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' \
    "$1/object.json" "$2"
}

# Compile one unit into ${OBJ}/<tag>.cfo against zero or more interface objects,
# each named by its tag. `--interface` reads only that object's object.json — an
# interface's chunk files are never opened.
compile_object() {
  local tag="$1" src="$2"; shift 2
  local -a interfaces=()
  local i
  for i in "$@"; do interfaces+=(--interface "${OBJ}/${i}.cfo"); done
  "${COMPILER}" compile-object --source "${src}" "${interfaces[@]}" \
    --out "${OBJ}/${tag}.cfo" > "${OBJ}/${tag}.log"
  [[ -f "${OBJ}/${tag}.cfo/object.json" ]] \
    || { echo "compile-object ${tag}: wrote no object.json" >&2; exit 1; }
  ok "$(object_field "${OBJ}/${tag}.cfo" unit): object_hash $(object_field "${OBJ}/${tag}.cfo" object_hash)"
}

# The catalogue declares what the three services read, so it compiles first and
# against nothing at all. Each service then compiles against its header.
compile_object site_catalogue "${CATALOGUE}"
compile_object vision_service "${VISION}" site_catalogue
compile_object compute_service "${COMPUTE}" site_catalogue
compile_object sorter_service "${SORTER}" site_catalogue

# Link the four objects, and record what was linked. `--write-lock` pins the
# object hash of every unit: a later link given `--lock` must reproduce exactly
# this set or be refused. The compiler CHECKS pins and fetches nothing — the
# checkout, the submodule, the artifact store or the CI job brings the objects.
LOCK="${OUT_DIR}/configflux.lock"
"${COMPILER}" link \
  --object "${OBJ}/site_catalogue.cfo" \
  --object "${OBJ}/vision_service.cfo" \
  --object "${OBJ}/compute_service.cfo" \
  --object "${OBJ}/sorter_service.cfo" \
  --out "${OUT_DIR}/linked" --write-lock "${LOCK}" > "${OUT_DIR}/link.log"
sed 's/^/     /' "${OUT_DIR}/link.log"
expect_equal "the lock pins every unit, by its package name" \
  "compute_service site_catalogue sorter_service vision_service" \
  "$(python3 -c 'import json,sys
print(" ".join(sorted(json.load(open(sys.argv[1]))["objects"])))' "${LOCK}")"
ok "linked 4 objects; their hashes are pinned in $(basename "${LOCK}")"

# The oracle. There is one code path, so the two forms cannot disagree: every
# file of the package — the chunks, the index, the manifest, the provenance
# sidecar and the compiled constraint model — is byte for byte the same.
assert_directories_byte_identical() {
  local tag="$1" what="$2" a="$3" b="$4" rel
  ( cd "${a}" && find . -type f | sort ) > "${OUT_DIR}/${tag}.a.list"
  ( cd "${b}" && find . -type f | sort ) > "${OUT_DIR}/${tag}.b.list"
  cmp -s "${OUT_DIR}/${tag}.a.list" "${OUT_DIR}/${tag}.b.list" \
    || { echo "${what}: the two packages hold different files" >&2; exit 1; }
  while read -r rel; do
    cmp -s "${a}/${rel}" "${b}/${rel}" \
      || { echo "${what}: ${rel} differs between the two packages" >&2; exit 1; }
  done < "${OUT_DIR}/${tag}.a.list"
  ok "${what}: $(wc -l < "${OUT_DIR}/${tag}.a.list" | tr -d ' ') files, byte for byte"
}

assert_objects_link_byte_identical() {
  expect_equal "linked model_hash" "${BASE_HASH}" \
    "$(awk '/^model_hash:/ {print $2}' "${OUT_DIR}/link.log")"
  assert_directories_byte_identical linked \
    "one-shot compile vs the linked objects" \
    "${OUT_DIR}/cmp" "${OUT_DIR}/linked"
}
assert_objects_link_byte_identical

# Two mistakes BETWEEN units. Neither is visible while a unit is compiled on its
# own — an unresolved reference is recorded in the header rather than rejected —
# and both are named at link time, before a single byte is written.
assert_link_refused_by() {
  local label="$1" code="$2"; shift 2
  local out="${OUT_DIR}/link-${label}"
  local err="${OUT_DIR}/link-${label}.stderr"
  local -a objects=()
  local o rc=0
  for o in "$@"; do objects+=(--object "${OBJ}/${o}.cfo"); done
  "${COMPILER}" link "${objects[@]}" --out "${out}" > /dev/null 2> "${err}" || rc=$?
  expect_equal "${label}: link exit code" "2" "${rc}"
  expect_grep "${label}: link diagnostic" "${code}" "${err}"
  [[ ! -e "${out}" ]] \
    || { echo "${label}: a refused link still wrote ${out}" >&2; exit 1; }
  sed 's/^/     /' "${err}"
  ok "${label}: refused (exit 2) as ${code}, wrote nothing"
}

banner "Step 7a: the catalogue object is missing from the link"
assert_link_refused_by no-catalogue E_LINK_UNRESOLVED_IMPORT \
  vision_service compute_service sorter_service

# The catalogue moved on after the services were compiled against it. Every
# object records the hash of each interface it was built against, so a service
# carrying a stale one is refused by name rather than linked quietly.
banner "Step 7b: a service compiled against an older catalogue object"
EDITED_CATALOGUE="${OUT_DIR}/patched/00_catalogue.json"
python3 -c 'import json,sys
chunk=json.load(open(sys.argv[1]))
chunk["catalogues"]["containers"]["entries"]["c3"]["height_mm"]=450
json.dump(chunk,open(sys.argv[2],"w"),indent=4)' "${CATALOGUE}" "${EDITED_CATALOGUE}"
compile_object site_catalogue_next "${EDITED_CATALOGUE}"
assert_link_refused_by stale-interface E_LINK_INTERFACE_MISMATCH \
  site_catalogue_next vision_service

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
banner "Done"
echo "All outputs are in ${OUT_DIR}/"
echo ""
echo "Key things to notice:"
echo "  - ONE model, FOUR repositories. The compiler takes each repository's"
echo "    exported chunk as a --source; ids must be unique across all of them,"
echo "    and each chunk's package names the unit it belongs to."
echo "  - A catalogue is typed data, declared wherever it belongs: the shared"
echo "    containers in the catalogue repository, the lane profiles in the"
echo "    sorter's own."
echo "  - Sameness is by BINDING, not by catalogue. Vision and compute name one"
echo "    binding and get one entry by construction; the sorter names another"
echo "    over the same table and gets its own — and reads it, like they do,"
echo "    inside its own requires block, naming no catalogue."
echo "  - Every mistake is named: the derive table, the accepts list, the"
echo "    equality constraint, and a facet bound twice each refuse with the"
echo "    rule or the input that caused them."
echo "  - The environment states only its FREE decisions. line_container is"
echo "    never written down anywhere but the derive table."
echo "  - One model, two ways to build it. Handing every chunk to one compile"
echo "    is the shortcut for a checkout that holds them all; compiling each"
echo "    repository into an object and linking the objects is the same code"
echo "    path, produces the same bytes, and is what four separate checkouts"
echo "    can actually do. Only then does a stale or missing unit have a name."
