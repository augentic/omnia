# JsonDb Example

Demonstrates `wasi:jsondb` using the default PoloDB backend with three GTFS-like collections (stops, routes, stop_times). Exercises all CRUD operations and most filter types through combined query endpoints.

## Quick Start

```bash
# build the guest
cargo build --example jsondb-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,omnia_wasi_jsondb=debug,omnia_wasi_http=debug"
cargo run --example jsondb -- run ./target/wasm32-wasip2/debug/examples/jsondb_wasm.wasm
```

## API Endpoints

### Stops

**Create stops**

```bash
# Britomart -- station, accessible, zone-1
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-001","stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1","wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-19T10:00:00Z"}'

# Newmarket -- stop, accessible, zone-1
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-002","stop_name":"Newmarket Station","stop_lat":-36.8690,"stop_lon":174.7779,"zone_id":"zone-1","wheelchair_boarding":1,"location_type":0,"parent_station":"stop-001","last_updated":"2026-03-19T10:00:00Z"}'

# Ponsonby -- stop, not accessible, zone-2
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-003","stop_name":"Ponsonby Rd at Franklin Rd","stop_lat":-36.8556,"stop_lon":174.7437,"zone_id":"zone-2","wheelchair_boarding":0,"location_type":0,"parent_station":null,"last_updated":"2026-03-18T08:00:00Z"}'

# Albany -- stop, accessible, zone-3
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-004","stop_name":"Albany Station","stop_lat":-36.7275,"stop_lon":174.6986,"zone_id":"zone-3","wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-17T09:00:00Z"}'

# Devonport -- stop, no zone, accessible
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-005","stop_name":"Devonport Ferry Terminal","stop_lat":-36.8326,"stop_lon":174.7950,"zone_id":null,"wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-19T11:00:00Z"}'
```

**Get a stop by ID**

```bash
curl -s http://localhost:8080/stops/stop-001
```

**Update a stop (upsert)**

```bash
curl -s -X PUT http://localhost:8080/stops/stop-001 \
  -H 'Content-Type: application/json' \
  -d '{"stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1","wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-19T12:00:00Z"}'
```

**Delete a stop**

```bash
curl -s -X DELETE http://localhost:8080/stops/stop-005
```

**Query stops**

```bash
# All stops (sorted by name)
curl -s "http://localhost:8080/stops"

# Text search -- contains on stop_name
curl -s "http://localhost:8080/stops?q=Station"

# By zone -- eq on zone_id
curl -s "http://localhost:8080/stops?zone=zone-1"

# Exclude a zone -- ne on zone_id (direct ComparisonOp::Ne codepath)
curl -s "http://localhost:8080/stops?exclude_zone=zone-1"

# Accessible stops -- eq(wheelchair_boarding, 1) + is_not_null(zone_id)
curl -s "http://localhost:8080/stops?accessible=true"

# Top-level stops only -- is_null(parent_station)
curl -s "http://localhost:8080/stops?top_level=true"

# Bounding box (Auckland CBD) -- and(gte, lte, gte, lte)
curl -s "http://localhost:8080/stops?min_lat=-36.86&max_lat=-36.84&min_lon=174.74&max_lon=174.80"

# Updated on date -- on_date(last_updated)
curl -s "http://localhost:8080/stops?updated_on=2026-03-19"

# Combined: accessible + zone + limit
curl -s "http://localhost:8080/stops?accessible=true&zone=zone-1&limit=5"

# Pagination
curl -s "http://localhost:8080/stops?limit=2"
# then use the returned continuation token:
# curl -s "http://localhost:8080/stops?limit=2&continuation=<token>"
```

### Routes

**Create routes**

```bash
# Northern Express -- bus
curl -s -X POST http://localhost:8080/routes \
  -H 'Content-Type: application/json' \
  -d '{"id":"route-nex","agency_id":"AT","route_short_name":"NEX","route_long_name":"Northern Express","route_type":3,"route_color":"00AEEF"}'

# Eastern Line -- rail
curl -s -X POST http://localhost:8080/routes \
  -H 'Content-Type: application/json' \
  -d '{"id":"route-east","agency_id":"AT","route_short_name":"EAST","route_long_name":"Eastern Line","route_type":2,"route_color":"EE4D2D"}'

# Devonport Ferry -- ferry
curl -s -X POST http://localhost:8080/routes \
  -H 'Content-Type: application/json' \
  -d '{"id":"route-dev","agency_id":"Fullers","route_short_name":"DEV","route_long_name":"Devonport Ferry","route_type":4,"route_color":"1D4F91"}'

# Inner Link -- bus
curl -s -X POST http://localhost:8080/routes \
  -H 'Content-Type: application/json' \
  -d '{"id":"route-ilk","agency_id":"AT","route_short_name":"ILK","route_long_name":"Inner Link","route_type":3,"route_color":"8BC53F"}'
```

**Get a route by ID**

```bash
curl -s http://localhost:8080/routes/route-nex
```

**Query routes**

```bash
# All routes
curl -s "http://localhost:8080/routes"

# Name search -- or(contains(short_name), contains(long_name))
curl -s "http://localhost:8080/routes?q=Northern"

# By route types -- in_list(route_type, [2, 3])
curl -s "http://localhost:8080/routes?types=2,3"

# By agency -- eq(agency_id)
curl -s "http://localhost:8080/routes?agency=AT"

# Exclude ferries -- not(eq(route_type, 4))
curl -s "http://localhost:8080/routes?exclude_type=4"

# Exclude AT buses -- not(and(eq(agency, AT), eq(type, 3))) (De Morgan negation)
curl -s "http://localhost:8080/routes?not_agency=AT&not_type=3"

# Combined: AT bus and rail, no ferries
curl -s "http://localhost:8080/routes?agency=AT&types=2,3&exclude_type=4"
```

### Stop Times

**Create stop times**

```bash
# NEX trip: stop-004 -> stop-001
curl -s -X POST http://localhost:8080/stop-times \
  -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-1","trip_id":"trip-nex-0800","stop_id":"stop-004","arrival_time":"08:00:00","departure_time":"08:01:00","stop_sequence":1,"pickup_type":0,"drop_off_type":0}'

curl -s -X POST http://localhost:8080/stop-times \
  -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-2","trip_id":"trip-nex-0800","stop_id":"stop-003","arrival_time":"08:15:00","departure_time":"08:16:00","stop_sequence":2,"pickup_type":0,"drop_off_type":0}'

curl -s -X POST http://localhost:8080/stop-times \
  -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-3","trip_id":"trip-nex-0800","stop_id":"stop-001","arrival_time":"08:30:00","departure_time":"08:31:00","stop_sequence":3,"pickup_type":0,"drop_off_type":1}'

# Eastern line trip
curl -s -X POST http://localhost:8080/stop-times \
  -H 'Content-Type: application/json' \
  -d '{"id":"east-0900-1","trip_id":"trip-east-0900","stop_id":"stop-001","arrival_time":"09:00:00","departure_time":"09:01:00","stop_sequence":1,"pickup_type":0,"drop_off_type":0}'

curl -s -X POST http://localhost:8080/stop-times \
  -H 'Content-Type: application/json' \
  -d '{"id":"east-0900-2","trip_id":"trip-east-0900","stop_id":"stop-002","arrival_time":"09:10:00","departure_time":"09:11:00","stop_sequence":2,"pickup_type":0,"drop_off_type":0}'
```

**Get a stop time by ID**

```bash
curl -s http://localhost:8080/stop-times/nex-0800-1
```

**Query stop times**

```bash
# All stop times for a trip -- eq(trip_id)
curl -s "http://localhost:8080/stop-times?trip=trip-nex-0800"

# Stop times at a stop -- eq(stop_id)
curl -s "http://localhost:8080/stop-times?stop=stop-001"

# Time range -- gte + lte on arrival_time
curl -s "http://localhost:8080/stop-times?after=08:00:00&before=08:30:00"

# Trip + sequence range -- eq + gte + lte
curl -s "http://localhost:8080/stop-times?trip=trip-nex-0800&min_seq=1&max_seq=2"

# Combined: stop + time range
curl -s "http://localhost:8080/stop-times?stop=stop-001&after=08:00:00&before=10:00:00"
```

## Features Demonstrated

- **CRUD** -- insert, get, put (upsert), delete on stops; insert + get on routes and stop_times
- **Combined query endpoints** -- each collection has one query endpoint that builds `Filter::and(...)` from whichever query params are present
- **Filter::eq** -- zone, agency, trip_id, stop_id
- **Filter::ne** -- exclude a specific zone (direct `ComparisonOp::Ne` codepath)
- **Filter::gte / lte** -- bounding box (lat/lon), time range, sequence range
- **Filter::contains** -- text search on stop_name
- **Filter::in_list** -- route types (bus, rail, ferry)
- **Filter::is_not_null** -- accessible stops require a zone
- **Filter::is_null** -- top-level stops (no parent_station)
- **Filter::or** -- route name search across short and long names
- **Filter::starts_with** -- route long name prefix search
- **Filter::not** -- exclude a route type
- **Filter::not(Filter::and(...))** -- exclude a specific agency+type combo (De Morgan negation)
- **Filter::on_date** -- stops updated on a specific date
- **Pagination** -- limit + continuation token (page 2 verified)
- **Sort** -- results sorted by name or sequence

## Test Script

Requires `jq`. Run with the server already started.

```bash
#!/usr/bin/env bash
set -euo pipefail

BASE="http://localhost:8080"
PASS=0; FAIL=0

check() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$actual" = "$expected" ]; then
    echo "PASS: $desc"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $desc (expected $expected, got $actual)"
    FAIL=$((FAIL + 1))
  fi
}

check_gte() {
  local desc="$1" min="$2" actual="$3"
  if [ "$actual" -ge "$min" ]; then
    echo "PASS: $desc ($actual >= $min)"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $desc (expected >= $min, got $actual)"
    FAIL=$((FAIL + 1))
  fi
}

echo "=== Seeding stops ==="
curl -sf -X POST "$BASE/stops" -H 'Content-Type: application/json' \
  -d '{"id":"stop-001","stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1","wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-19T10:00:00Z"}' > /dev/null
curl -sf -X POST "$BASE/stops" -H 'Content-Type: application/json' \
  -d '{"id":"stop-002","stop_name":"Newmarket Station","stop_lat":-36.8690,"stop_lon":174.7779,"zone_id":"zone-1","wheelchair_boarding":1,"location_type":0,"parent_station":"stop-001","last_updated":"2026-03-19T10:00:00Z"}' > /dev/null
curl -sf -X POST "$BASE/stops" -H 'Content-Type: application/json' \
  -d '{"id":"stop-003","stop_name":"Ponsonby Rd at Franklin Rd","stop_lat":-36.8556,"stop_lon":174.7437,"zone_id":"zone-2","wheelchair_boarding":0,"location_type":0,"parent_station":null,"last_updated":"2026-03-18T08:00:00Z"}' > /dev/null
curl -sf -X POST "$BASE/stops" -H 'Content-Type: application/json' \
  -d '{"id":"stop-004","stop_name":"Albany Station","stop_lat":-36.7275,"stop_lon":174.6986,"zone_id":"zone-3","wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-17T09:00:00Z"}' > /dev/null
curl -sf -X POST "$BASE/stops" -H 'Content-Type: application/json' \
  -d '{"id":"stop-005","stop_name":"Devonport Ferry Terminal","stop_lat":-36.8326,"stop_lon":174.7950,"zone_id":null,"wheelchair_boarding":1,"location_type":1,"parent_station":null,"last_updated":"2026-03-19T11:00:00Z"}' > /dev/null

echo "=== Seeding routes ==="
curl -sf -X POST "$BASE/routes" -H 'Content-Type: application/json' \
  -d '{"id":"route-nex","agency_id":"AT","route_short_name":"NEX","route_long_name":"Northern Express","route_type":3,"route_color":"00AEEF"}' > /dev/null
curl -sf -X POST "$BASE/routes" -H 'Content-Type: application/json' \
  -d '{"id":"route-east","agency_id":"AT","route_short_name":"EAST","route_long_name":"Eastern Line","route_type":2,"route_color":"EE4D2D"}' > /dev/null
curl -sf -X POST "$BASE/routes" -H 'Content-Type: application/json' \
  -d '{"id":"route-dev","agency_id":"Fullers","route_short_name":"DEV","route_long_name":"Devonport Ferry","route_type":4,"route_color":"1D4F91"}' > /dev/null
curl -sf -X POST "$BASE/routes" -H 'Content-Type: application/json' \
  -d '{"id":"route-ilk","agency_id":"AT","route_short_name":"ILK","route_long_name":"Inner Link","route_type":3,"route_color":"8BC53F"}' > /dev/null

echo "=== Seeding stop times ==="
curl -sf -X POST "$BASE/stop-times" -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-1","trip_id":"trip-nex-0800","stop_id":"stop-004","arrival_time":"08:00:00","departure_time":"08:01:00","stop_sequence":1,"pickup_type":0,"drop_off_type":0}' > /dev/null
curl -sf -X POST "$BASE/stop-times" -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-2","trip_id":"trip-nex-0800","stop_id":"stop-003","arrival_time":"08:15:00","departure_time":"08:16:00","stop_sequence":2,"pickup_type":0,"drop_off_type":0}' > /dev/null
curl -sf -X POST "$BASE/stop-times" -H 'Content-Type: application/json' \
  -d '{"id":"nex-0800-3","trip_id":"trip-nex-0800","stop_id":"stop-001","arrival_time":"08:30:00","departure_time":"08:31:00","stop_sequence":3,"pickup_type":0,"drop_off_type":1}' > /dev/null
curl -sf -X POST "$BASE/stop-times" -H 'Content-Type: application/json' \
  -d '{"id":"east-0900-1","trip_id":"trip-east-0900","stop_id":"stop-001","arrival_time":"09:00:00","departure_time":"09:01:00","stop_sequence":1,"pickup_type":0,"drop_off_type":0}' > /dev/null
curl -sf -X POST "$BASE/stop-times" -H 'Content-Type: application/json' \
  -d '{"id":"east-0900-2","trip_id":"trip-east-0900","stop_id":"stop-002","arrival_time":"09:10:00","departure_time":"09:11:00","stop_sequence":2,"pickup_type":0,"drop_off_type":0}' > /dev/null

echo ""
echo "=== Testing stops ==="

R=$(curl -s "$BASE/stops/stop-001")
check "get stop by id" "Britomart Transport Centre" "$(echo "$R" | jq -r '.stop.stop_name')"

R=$(curl -s "$BASE/stops")
check_gte "list all stops" 5 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?q=Station")
check_gte "text search (contains)" 2 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?zone=zone-1")
check "zone filter (eq)" "2" "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?accessible=true")
check_gte "accessible filter (eq + is_not_null)" 3 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?top_level=true")
check_gte "top-level filter (is_null)" 3 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?min_lat=-36.86&max_lat=-36.83&min_lon=174.74&max_lon=174.80")
check_gte "bounding box (and + gte + lte)" 2 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?updated_on=2026-03-19")
check_gte "updated on date (on_date)" 3 "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?exclude_zone=zone-1")
check "exclude zone (ne)" "3" "$(echo "$R" | jq '.stops | length')"

R=$(curl -s "$BASE/stops?limit=2")
check "pagination limit" "2" "$(echo "$R" | jq '.stops | length')"

TOKEN=$(echo "$R" | jq -r '.continuation // empty')
if [ -n "$TOKEN" ]; then
  R2=$(curl -s "$BASE/stops?limit=2&continuation=$TOKEN")
  PAGE2=$(echo "$R2" | jq '.stops | length')
  check_gte "pagination page 2 (continuation)" 1 "$PAGE2"
else
  echo "FAIL: no continuation token returned for limit=2"
  FAIL=$((FAIL + 1))
fi

echo ""
echo "=== Testing routes ==="

R=$(curl -s "$BASE/routes/route-nex")
check "get route by id" "NEX" "$(echo "$R" | jq -r '.route.route_short_name')"

R=$(curl -s "$BASE/routes?q=Northern")
check_gte "name search (or + contains + starts_with)" 1 "$(echo "$R" | jq '.routes | length')"

R=$(curl -s "$BASE/routes?types=2,3")
check "in_list route types" "3" "$(echo "$R" | jq '.routes | length')"

R=$(curl -s "$BASE/routes?agency=AT")
check "agency filter (eq)" "3" "$(echo "$R" | jq '.routes | length')"

R=$(curl -s "$BASE/routes?exclude_type=4")
check "exclude ferries (not)" "3" "$(echo "$R" | jq '.routes | length')"

R=$(curl -s "$BASE/routes?not_agency=AT&not_type=3")
check "exclude AT buses (not+and de morgan)" "2" "$(echo "$R" | jq '.routes | length')"

echo ""
echo "=== Testing stop times ==="

R=$(curl -s "$BASE/stop-times/nex-0800-1")
check "get stop_time by id" "trip-nex-0800" "$(echo "$R" | jq -r '.stop_time.trip_id')"

R=$(curl -s "$BASE/stop-times?trip=trip-nex-0800")
check "trip filter (eq)" "3" "$(echo "$R" | jq '.stop_times | length')"

R=$(curl -s "$BASE/stop-times?stop=stop-001")
check "stop filter (eq)" "2" "$(echo "$R" | jq '.stop_times | length')"

R=$(curl -s "$BASE/stop-times?after=08:00:00&before=08:30:00")
check_gte "time range (gte + lte)" 2 "$(echo "$R" | jq '.stop_times | length')"

R=$(curl -s "$BASE/stop-times?trip=trip-nex-0800&min_seq=1&max_seq=2")
check "trip + seq range" "2" "$(echo "$R" | jq '.stop_times | length')"

echo ""
echo "=== Cleanup ==="
curl -sf -X DELETE "$BASE/stops/stop-005" > /dev/null && echo "deleted stop-005"

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
```
