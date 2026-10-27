//! Bounded geospatial commands stored as sorted-set geohash scores.
//!
//! Coordinates are encoded into the same 52-bit interleaved geohash score that
//! Redis uses for geo indexes, so TYPE/COPY/RENAME/expiry/sorted-set commands
//! apply to geo keys. Searches scan the bounded sorted-set payload linearly
//! instead of walking nine neighbouring geohash cells; results and distances use
//! the Redis WGS-84 formulas, but very large sets trade the native index for a
//! simpler, allocation-bounded implementation.

use super::collection_commands::encode_zset;
use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{Store, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::collections::HashMap;
use std::io::Write;

const EARTH_RADIUS_METERS: f64 = 6_372_797.560_856;
const GEO_LAT_MIN: f64 = -85.05112878;
const GEO_LAT_MAX: f64 = 85.05112878;
const GEO_LONG_MIN: f64 = -180.0;
const GEO_LONG_MAX: f64 = 180.0;
const GEO_STEP: u32 = 26;
const GEO_ALPHABET: &[u8; 32] = b"0123456789bcdefghjkmnpqrstuvwxyz";
const MAX_ELEMENTS: usize = 1_000_000;

const SYNTAX_ERROR: &[u8] = b"-ERR syntax error\r\n";
const FLOAT_ERROR: &[u8] = b"-ERR value is not a valid float\r\n";
const UNIT_ERROR: &[u8] = b"-ERR unsupported unit provided. please use M, KM, FT, MI\r\n";
const MEMBER_ERROR: &[u8] = b"-ERR could not decode requested zset member\r\n";
const ANY_ERROR: &[u8] = b"-ERR the ANY argument requires COUNT argument\r\n";
const COUNT_ERROR: &[u8] = b"-ERR COUNT must be > 0\r\n";
const STORE_WITH_ERROR: &[u8] =
    b"-ERR GEOSEARCHSTORE is not compatible with WITHDIST, WITHHASH and WITHCOORD options\r\n";
const LEGACY_STORE_WITH_ERROR: &[u8] =
    b"-ERR STORE option in GEORADIUS is not compatible with WITHDIST, WITHHASH and WITHCOORD options\r\n";

#[derive(Clone, Copy)]
enum Sort {
    Asc,
    Desc,
}

enum FromSpec {
    Member(Vec<u8>),
    LonLat(f64, f64),
}

enum Shape {
    Radius(f64),
    Box { width: f64, height: f64 },
}

struct Search {
    from: FromSpec,
    shape: Shape,
    sort: Option<Sort>,
    count: Option<usize>,
    any: bool,
    with_coord: bool,
    with_dist: bool,
    with_hash: bool,
    storedist: bool,
    conversion: f64,
}

struct Hit {
    member: Vec<u8>,
    distance: f64,
    hash: u64,
    lon: f64,
    lat: f64,
}

impl Store {
    pub(super) fn execute_geo(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command
            .get(..3)
            .is_none_or(|prefix| !prefix.eq_ignore_ascii_case(b"GEO"))
        {
            return false;
        }
        if command.eq_ignore_ascii_case(b"GEOADD") {
            if args.len() < 5 {
                arity("geoadd", out);
            } else {
                self.geoadd(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEODIST") {
            if args.len() < 4 || args.len() > 5 {
                arity("geodist", out);
            } else {
                self.geodist(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEOHASH") {
            if args.len() < 2 {
                arity("geohash", out);
            } else {
                self.geohash(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEOPOS") {
            if args.len() < 2 {
                arity("geopos", out);
            } else {
                self.geopos(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEOSEARCH") {
            if args.len() < 7 {
                arity("geosearch", out);
            } else {
                self.geosearch(args, false, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEOSEARCHSTORE") {
            if args.len() < 8 {
                arity("geosearchstore", out);
            } else {
                self.geosearch(args, true, out);
            }
        } else if command.eq_ignore_ascii_case(b"GEORADIUS")
            || command.eq_ignore_ascii_case(b"GEORADIUS_RO")
            || command.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER")
            || command.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER_RO")
        {
            let by_member = command.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER")
                || command.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER_RO");
            let readonly = command.eq_ignore_ascii_case(b"GEORADIUS_RO")
                || command.eq_ignore_ascii_case(b"GEORADIUSBYMEMBER_RO");
            let name = if by_member {
                if readonly {
                    "georadiusbymember_ro"
                } else {
                    "georadiusbymember"
                }
            } else if readonly {
                "georadius_ro"
            } else {
                "georadius"
            };
            if args.len() < if by_member { 5 } else { 6 } {
                arity(name, out);
            } else if let Some((search, destination)) = parse_legacy(args, by_member, readonly, out)
            {
                self.run_search(args[1], destination, &search, out);
            }
        } else {
            return false;
        }
        true
    }

    fn geoadd(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut nx = false;
        let mut xx = false;
        let mut ch = false;
        let mut index = 2;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"NX") {
                nx = true;
            } else if args[index].eq_ignore_ascii_case(b"XX") {
                xx = true;
            } else if args[index].eq_ignore_ascii_case(b"CH") {
                ch = true;
            } else {
                break;
            }
            index += 1;
        }
        if nx && xx || index >= args.len() || !(args.len() - index).is_multiple_of(3) {
            out.extend_from_slice(SYNTAX_ERROR);
            return;
        }
        let mut rewritten = Vec::with_capacity(3 + (args.len() - index) / 3 * 2);
        rewritten.push(b"ZADD".to_vec());
        rewritten.push(args[1].to_vec());
        if nx {
            rewritten.push(b"NX".to_vec());
        }
        if xx {
            rewritten.push(b"XX".to_vec());
        }
        if ch {
            rewritten.push(b"CH".to_vec());
        }
        while index < args.len() {
            let Some((lon, lat)) = parse_lonlat(args[index], args[index + 1]) else {
                out.extend_from_slice(FLOAT_ERROR);
                return;
            };
            if !valid_lonlat(lon, lat) {
                coordinate_error(lon, lat, out);
                return;
            }
            rewritten.push(interleave_geohash(lon, lat).to_string().into_bytes());
            rewritten.push(args[index + 2].to_vec());
            index += 3;
        }
        let refs: Vec<&[u8]> = rewritten.iter().map(Vec::as_slice).collect();
        self.execute_collection_commands(&refs, out);
    }

    fn geopos(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let members = match data.entries.get(args[1]) {
            None => None,
            Some(entry)
                if entry
                    .collection
                    .as_ref()
                    .is_some_and(|value| value.is_zset()) =>
            {
                entry
                    .collection
                    .as_ref()
                    .and_then(|value| value.zset_items())
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        write!(out, "*{}\r\n", args.len() - 2).expect("Vec write");
        for member in &args[2..] {
            let coordinates = members
                .as_ref()
                .and_then(|members| members.iter().find(|(name, _)| name == member))
                .and_then(|(_, score)| decode_geohash(*score));
            match coordinates {
                Some((lon, lat)) => {
                    out.extend_from_slice(b"*2\r\n");
                    encode_bulk(Some(format_human(lon).as_bytes()), out);
                    encode_bulk(Some(format_human(lat).as_bytes()), out);
                }
                None => out.extend_from_slice(b"*-1\r\n"),
            }
        }
    }

    fn geodist(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let conversion = if args.len() == 5 {
            let Some(factor) = unit_factor(args[4]) else {
                out.extend_from_slice(UNIT_ERROR);
                return;
            };
            factor
        } else {
            1.0
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let members = match data.entries.get(args[1]) {
            None => {
                encode_bulk(None, out);
                return;
            }
            Some(entry)
                if entry
                    .collection
                    .as_ref()
                    .is_some_and(|value| value.is_zset()) =>
            {
                entry
                    .collection
                    .as_ref()
                    .and_then(|value| value.zset_items())
                    .expect("zset type checked")
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let score = |member: &[u8]| {
            members
                .iter()
                .find(|(name, _)| *name == member)
                .map(|(_, score)| *score)
        };
        let (Some(first), Some(second)) = (score(args[2]), score(args[3])) else {
            encode_bulk(None, out);
            return;
        };
        let (Some((lon1, lat1)), Some((lon2, lat2))) =
            (decode_geohash(first), decode_geohash(second))
        else {
            encode_bulk(None, out);
            return;
        };
        let distance = haversine(lon1, lat1, lon2, lat2) / conversion;
        encode_bulk(Some(format_distance(distance).as_bytes()), out);
    }

    fn geohash(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let members = match data.entries.get(args[1]) {
            None => None,
            Some(entry)
                if entry
                    .collection
                    .as_ref()
                    .is_some_and(|value| value.is_zset()) =>
            {
                entry
                    .collection
                    .as_ref()
                    .and_then(|value| value.zset_items())
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        write!(out, "*{}\r\n", args.len() - 2).expect("Vec write");
        for member in &args[2..] {
            let hash = members
                .as_ref()
                .and_then(|members| members.iter().find(|(name, _)| name == member))
                .and_then(|(_, score)| decode_geohash(*score))
                .map(|(lon, lat)| standard_geohash(lon, lat));
            encode_bulk(hash.as_deref(), out);
        }
    }

    fn geosearch(&self, args: &[&[u8]], store: bool, out: &mut Vec<u8>) {
        let Some(search) = parse_search(args, store, out) else {
            return;
        };
        if store {
            self.run_search(args[2], Some(args[1]), &search, out);
        } else {
            self.run_search(args[1], None, &search, out);
        }
    }

    fn run_search(
        &self,
        source: &[u8],
        destination: Option<&[u8]>,
        search: &Search,
        out: &mut Vec<u8>,
    ) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(source, now);
        let members = match data.entries.get(source) {
            None => None,
            Some(entry)
                if entry
                    .collection
                    .as_ref()
                    .is_some_and(|value| value.is_zset()) =>
            {
                entry
                    .collection
                    .as_ref()
                    .and_then(|value| value.zset_items())
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let Some(members) = members else {
            if let Some(destination) = destination {
                data.expire_key(destination, now);
                data.remove(destination);
                integer(0, out);
            } else {
                out.extend_from_slice(b"*0\r\n");
            }
            return;
        };
        let center = match &search.from {
            FromSpec::LonLat(lon, lat) => (*lon, *lat),
            FromSpec::Member(member) => {
                let Some(score) = members
                    .iter()
                    .find(|(name, _)| *name == member.as_slice())
                    .map(|(_, score)| *score)
                else {
                    out.extend_from_slice(MEMBER_ERROR);
                    return;
                };
                let Some(coordinates) = decode_geohash(score) else {
                    out.extend_from_slice(MEMBER_ERROR);
                    return;
                };
                coordinates
            }
        };
        let mut hits = Vec::new();
        for (member, score) in members {
            let Some((lon, lat)) = decode_geohash(score) else {
                continue;
            };
            let distance = haversine(center.0, center.1, lon, lat);
            let inside = match search.shape {
                Shape::Radius(radius) => distance <= radius,
                Shape::Box { width, height } => {
                    let latitude_distance = haversine(center.0, center.1, center.0, lat);
                    latitude_distance <= height / 2.0
                        && haversine(center.0, lat, lon, lat) <= width / 2.0
                }
            };
            if !inside {
                continue;
            }
            hits.push(Hit {
                member: member.to_vec(),
                distance,
                hash: score as u64,
                lon,
                lat,
            });
            if search.any && search.count.is_some_and(|count| hits.len() >= count) {
                break;
            }
        }
        if search.any {
            if let Some(sort) = search.sort {
                sort_hits(&mut hits, sort);
            } else {
                hits.sort_unstable_by(|a, b| {
                    a.hash.cmp(&b.hash).then_with(|| a.member.cmp(&b.member))
                });
            }
        } else {
            match search.sort {
                Some(sort) => sort_hits(&mut hits, sort),
                None => hits.sort_unstable_by(|a, b| {
                    a.hash.cmp(&b.hash).then_with(|| a.member.cmp(&b.member))
                }),
            }
        }
        if let Some(count) = search.count {
            hits.truncate(count);
        }
        if let Some(destination) = destination {
            self.store_search(destination, hits, search, &mut data, now, out);
            return;
        }
        let options = usize::from(search.with_dist)
            + usize::from(search.with_hash)
            + usize::from(search.with_coord);
        write!(out, "*{}\r\n", hits.len()).expect("Vec write");
        for hit in hits {
            if options > 0 {
                write!(out, "*{}\r\n", options + 1).expect("Vec write");
            }
            encode_bulk(Some(&hit.member), out);
            if search.with_dist {
                encode_bulk(
                    Some(format_distance(hit.distance / search.conversion).as_bytes()),
                    out,
                );
            }
            if search.with_hash {
                integer(hit.hash as i64, out);
            }
            if search.with_coord {
                out.extend_from_slice(b"*2\r\n");
                encode_bulk(Some(format_human(hit.lon).as_bytes()), out);
                encode_bulk(Some(format_human(hit.lat).as_bytes()), out);
            }
        }
    }

    fn store_search(
        &self,
        destination: &[u8],
        hits: Vec<Hit>,
        search: &Search,
        data: &mut super::data::Data,
        now: i64,
        out: &mut Vec<u8>,
    ) {
        data.expire_key(destination, now);
        if hits.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        let mut zset = HashMap::with_capacity(hits.len());
        let mut bytes = 0usize;
        for hit in hits {
            bytes = bytes.saturating_add(hit.member.len());
            let score = if search.storedist {
                hit.distance / search.conversion
            } else {
                hit.hash as f64
            };
            zset.insert(hit.member, score);
        }
        if zset.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let stored = zset.len();
        let previous = data.entries.get(destination).map(|entry| entry.payload_len);
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(destination, bytes, previous, self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(destination, encoded, None, payload, now);
        integer(stored as i64, out);
    }
}

fn sort_hits(hits: &mut [Hit], sort: Sort) {
    match sort {
        Sort::Asc => hits.sort_unstable_by(|a, b| {
            a.distance
                .total_cmp(&b.distance)
                .then_with(|| a.member.cmp(&b.member))
        }),
        Sort::Desc => hits.sort_unstable_by(|a, b| {
            b.distance
                .total_cmp(&a.distance)
                .then_with(|| a.member.cmp(&b.member))
        }),
    }
}

fn parse_legacy<'a>(
    args: &'a [&'a [u8]],
    by_member: bool,
    readonly: bool,
    out: &mut Vec<u8>,
) -> Option<(Search, Option<&'a [u8]>)> {
    let mut index;
    let from;
    let radius;
    let unit;
    if by_member {
        let member = args[2];
        let Some(raw_radius) = args.get(3) else {
            out.extend_from_slice(SYNTAX_ERROR);
            return None;
        };
        let Some(raw_unit) = args.get(4) else {
            out.extend_from_slice(SYNTAX_ERROR);
            return None;
        };
        from = FromSpec::Member(member.to_vec());
        radius = raw_radius;
        unit = raw_unit;
        index = 5;
    } else {
        let (Some(raw_lon), Some(raw_lat), Some(raw_radius), Some(raw_unit)) =
            (args.get(2), args.get(3), args.get(4), args.get(5))
        else {
            out.extend_from_slice(SYNTAX_ERROR);
            return None;
        };
        let Some((lon, lat)) = parse_lonlat(raw_lon, raw_lat) else {
            out.extend_from_slice(FLOAT_ERROR);
            return None;
        };
        if !valid_lonlat(lon, lat) {
            coordinate_error(lon, lat, out);
            return None;
        }
        from = FromSpec::LonLat(lon, lat);
        radius = raw_radius;
        unit = raw_unit;
        index = 6;
    }
    let Some(factor) = unit_factor(unit) else {
        out.extend_from_slice(UNIT_ERROR);
        return None;
    };
    let Some(radius) = parse_number(radius) else {
        out.extend_from_slice(b"-ERR need numeric radius\r\n");
        return None;
    };
    if radius < 0.0 {
        out.extend_from_slice(b"-ERR radius cannot be negative\r\n");
        return None;
    }
    let mut sort = None;
    let mut count = None;
    let mut any = false;
    let mut with_coord = false;
    let mut with_dist = false;
    let mut with_hash = false;
    let mut destination = None;
    let mut storedist = false;
    while index < args.len() {
        let token = args[index];
        if token.eq_ignore_ascii_case(b"WITHCOORD") {
            with_coord = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"WITHDIST") {
            with_dist = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"WITHHASH") {
            with_hash = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"ASC") {
            sort = Some(Sort::Asc);
            index += 1;
        } else if token.eq_ignore_ascii_case(b"DESC") {
            sort = Some(Sort::Desc);
            index += 1;
        } else if token.eq_ignore_ascii_case(b"COUNT") {
            let Some(raw) = args.get(index + 1) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            let Some(value) = parse_integer(raw) else {
                out.extend_from_slice(NOT_INTEGER);
                return None;
            };
            if value <= 0 {
                out.extend_from_slice(COUNT_ERROR);
                return None;
            }
            count = Some(usize::try_from(value).unwrap_or(usize::MAX));
            index += 2;
        } else if token.eq_ignore_ascii_case(b"ANY") {
            any = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"STORE") || token.eq_ignore_ascii_case(b"STOREDIST") {
            if readonly {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            }
            let Some(name) = args.get(index + 1) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            destination = Some(*name);
            storedist = token.eq_ignore_ascii_case(b"STOREDIST");
            index += 2;
        } else {
            out.extend_from_slice(SYNTAX_ERROR);
            return None;
        }
    }
    if any && count.is_none() {
        out.extend_from_slice(ANY_ERROR);
        return None;
    }
    if destination.is_some() && (with_coord || with_dist || with_hash) {
        out.extend_from_slice(LEGACY_STORE_WITH_ERROR);
        return None;
    }
    if count.is_some() && sort.is_none() && !any {
        sort = Some(Sort::Asc);
    }
    Some((
        Search {
            from,
            shape: Shape::Radius(radius * factor),
            sort,
            count,
            any,
            with_coord,
            with_dist,
            with_hash,
            storedist,
            conversion: factor,
        },
        destination,
    ))
}

fn parse_search(args: &[&[u8]], store: bool, out: &mut Vec<u8>) -> Option<Search> {
    let mut from = None;
    let mut shape = None;
    let mut sort = None;
    let mut count = None;
    let mut any = false;
    let mut with_coord = false;
    let mut with_dist = false;
    let mut with_hash = false;
    let mut storedist = false;
    let mut conversion = 1.0;
    let mut index = if store { 3 } else { 2 };
    while index < args.len() {
        let token = args[index];
        if token.eq_ignore_ascii_case(b"FROMMEMBER") {
            if from.is_some() {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            }
            let Some(member) = args.get(index + 1) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            from = Some(FromSpec::Member(member.to_vec()));
            index += 2;
        } else if token.eq_ignore_ascii_case(b"FROMLONLAT") {
            if from.is_some() {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            }
            let (Some(lon), Some(lat)) = (args.get(index + 1), args.get(index + 2)) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            let Some((lon, lat)) = parse_lonlat(lon, lat) else {
                out.extend_from_slice(FLOAT_ERROR);
                return None;
            };
            if !valid_lonlat(lon, lat) {
                coordinate_error(lon, lat, out);
                return None;
            }
            from = Some(FromSpec::LonLat(lon, lat));
            index += 3;
        } else if token.eq_ignore_ascii_case(b"BYRADIUS") {
            if shape.is_some() {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            }
            let (Some(radius), Some(unit)) = (args.get(index + 1), args.get(index + 2)) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            let Some(factor) = unit_factor(unit) else {
                out.extend_from_slice(UNIT_ERROR);
                return None;
            };
            let Some(radius) = parse_number(radius) else {
                out.extend_from_slice(b"-ERR need numeric radius\r\n");
                return None;
            };
            if radius < 0.0 {
                out.extend_from_slice(b"-ERR radius cannot be negative\r\n");
                return None;
            }
            conversion = factor;
            shape = Some(Shape::Radius(radius * factor));
            index += 3;
        } else if token.eq_ignore_ascii_case(b"BYBOX") {
            if shape.is_some() {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            }
            let (Some(width), Some(height), Some(unit)) = (
                args.get(index + 1),
                args.get(index + 2),
                args.get(index + 3),
            ) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            let Some(factor) = unit_factor(unit) else {
                out.extend_from_slice(UNIT_ERROR);
                return None;
            };
            let Some(width) = parse_number(width) else {
                out.extend_from_slice(b"-ERR need numeric width\r\n");
                return None;
            };
            let Some(height) = parse_number(height) else {
                out.extend_from_slice(b"-ERR need numeric height\r\n");
                return None;
            };
            if width < 0.0 || height < 0.0 {
                out.extend_from_slice(b"-ERR height or width cannot be negative\r\n");
                return None;
            }
            conversion = factor;
            shape = Some(Shape::Box {
                width: width * factor,
                height: height * factor,
            });
            index += 4;
        } else if token.eq_ignore_ascii_case(b"ASC") {
            sort = Some(Sort::Asc);
            index += 1;
        } else if token.eq_ignore_ascii_case(b"DESC") {
            sort = Some(Sort::Desc);
            index += 1;
        } else if token.eq_ignore_ascii_case(b"COUNT") {
            let Some(raw) = args.get(index + 1) else {
                out.extend_from_slice(SYNTAX_ERROR);
                return None;
            };
            let Some(value) = parse_integer(raw) else {
                out.extend_from_slice(NOT_INTEGER);
                return None;
            };
            if value <= 0 {
                out.extend_from_slice(COUNT_ERROR);
                return None;
            }
            count = Some(usize::try_from(value).unwrap_or(usize::MAX));
            index += 2;
        } else if token.eq_ignore_ascii_case(b"ANY") {
            any = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"WITHCOORD") {
            with_coord = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"WITHDIST") {
            with_dist = true;
            index += 1;
        } else if token.eq_ignore_ascii_case(b"WITHHASH") {
            with_hash = true;
            index += 1;
        } else if store && token.eq_ignore_ascii_case(b"STOREDIST") {
            storedist = true;
            index += 1;
        } else {
            out.extend_from_slice(SYNTAX_ERROR);
            return None;
        }
    }
    let Some(from) = from else {
        out.extend_from_slice(if store {
            b"-ERR exactly one of FROMMEMBER or FROMLONLAT can be specified for GEOSEARCHSTORE\r\n"
        } else {
            b"-ERR exactly one of FROMMEMBER or FROMLONLAT can be specified for GEOSEARCH\r\n"
        });
        return None;
    };
    let Some(shape) = shape else {
        out.extend_from_slice(if store {
            b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCHSTORE\r\n"
        } else {
            b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCH\r\n"
        });
        return None;
    };
    if any && count.is_none() {
        out.extend_from_slice(ANY_ERROR);
        return None;
    }
    if store && (with_coord || with_dist || with_hash) {
        out.extend_from_slice(STORE_WITH_ERROR);
        return None;
    }
    if count.is_some() && sort.is_none() && !any {
        sort = Some(Sort::Asc);
    }
    Some(Search {
        from,
        shape,
        sort,
        count,
        any,
        with_coord,
        with_dist,
        with_hash,
        storedist,
        conversion,
    })
}

fn valid_lonlat(lon: f64, lat: f64) -> bool {
    lon.is_finite()
        && lat.is_finite()
        && (GEO_LONG_MIN..=GEO_LONG_MAX).contains(&lon)
        && (GEO_LAT_MIN..=GEO_LAT_MAX).contains(&lat)
}

fn coordinate_error(lon: f64, lat: f64, out: &mut Vec<u8>) {
    write!(
        out,
        "-ERR invalid longitude,latitude pair {lon:.6},{lat:.6}\r\n"
    )
    .expect("Vec write");
}

fn parse_lonlat(lon: &[u8], lat: &[u8]) -> Option<(f64, f64)> {
    Some((parse_number(lon)?, parse_number(lat)?))
}

fn parse_number(raw: &[u8]) -> Option<f64> {
    std::str::from_utf8(raw)
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|value| !value.is_nan())
}

fn unit_factor(raw: &[u8]) -> Option<f64> {
    if raw.eq_ignore_ascii_case(b"m") {
        Some(1.0)
    } else if raw.eq_ignore_ascii_case(b"km") {
        Some(1000.0)
    } else if raw.eq_ignore_ascii_case(b"ft") {
        Some(0.3048)
    } else if raw.eq_ignore_ascii_case(b"mi") {
        Some(1609.34)
    } else {
        None
    }
}

fn format_human(value: f64) -> String {
    let mut text = format!("{value:.17}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

fn format_distance(value: f64) -> String {
    format!("{value:.4}")
}

fn compact(mut value: u64) -> u32 {
    value &= 0x5555_5555_5555_5555;
    value = (value | (value >> 1)) & 0x3333_3333_3333_3333;
    value = (value | (value >> 2)) & 0x0f0f_0f0f_0f0f_0f0f;
    value = (value | (value >> 4)) & 0x00ff_00ff_00ff_00ff;
    value = (value | (value >> 8)) & 0x0000_ffff_0000_ffff;
    value = (value | (value >> 16)) & 0x0000_0000_ffff_ffff;
    value as u32
}

fn spread(mut value: u64) -> u64 {
    value &= 0xffff_ffff;
    value = (value | (value << 16)) & 0x0000_ffff_0000_ffff;
    value = (value | (value << 8)) & 0x00ff_00ff_00ff_00ff;
    value = (value | (value << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    value = (value | (value << 2)) & 0x3333_3333_3333_3333;
    value = (value | (value << 1)) & 0x5555_5555_5555_5555;
    value
}

fn interleave(lat: u32, lon: u32) -> u64 {
    spread(u64::from(lat)) | (spread(u64::from(lon)) << 1)
}

fn interleave_geohash(lon: f64, lat: f64) -> u64 {
    let scale = f64::from(1_u32 << GEO_STEP);
    let lat_offset = ((lat - GEO_LAT_MIN) / (GEO_LAT_MAX - GEO_LAT_MIN) * scale) as u32;
    let lon_offset = ((lon - GEO_LONG_MIN) / (GEO_LONG_MAX - GEO_LONG_MIN) * scale) as u32;
    interleave(lat_offset, lon_offset)
}

fn decode_geohash(score: f64) -> Option<(f64, f64)> {
    if !score.is_finite() || score < 0.0 {
        return None;
    }
    let bits = score as u64;
    let lat_bits = compact(bits);
    let lon_bits = compact(bits >> 1);
    let scale = f64::from(1_u32 << GEO_STEP);
    let lat_min = GEO_LAT_MIN + (f64::from(lat_bits) / scale) * (GEO_LAT_MAX - GEO_LAT_MIN);
    let lat_max = GEO_LAT_MIN + ((f64::from(lat_bits) + 1.0) / scale) * (GEO_LAT_MAX - GEO_LAT_MIN);
    let lon_min = GEO_LONG_MIN + (f64::from(lon_bits) / scale) * (GEO_LONG_MAX - GEO_LONG_MIN);
    let lon_max =
        GEO_LONG_MIN + ((f64::from(lon_bits) + 1.0) / scale) * (GEO_LONG_MAX - GEO_LONG_MIN);
    let lon = ((lon_min + lon_max) / 2.0).clamp(GEO_LONG_MIN, GEO_LONG_MAX);
    let lat = ((lat_min + lat_max) / 2.0).clamp(GEO_LAT_MIN, GEO_LAT_MAX);
    Some((lon, lat))
}

fn standard_geohash(lon: f64, lat: f64) -> Vec<u8> {
    let scale = f64::from(1_u32 << GEO_STEP);
    let lat_offset = ((lat + 90.0) / 180.0 * scale) as u32;
    let lon_offset = ((lon + 180.0) / 360.0 * scale) as u32;
    let bits = interleave(lat_offset, lon_offset);
    let mut output = Vec::with_capacity(11);
    for index in 0..11 {
        let slot = if index == 10 {
            0
        } else {
            ((bits >> (52 - ((index + 1) * 5))) & 0x1f) as usize
        };
        output.push(GEO_ALPHABET[slot]);
    }
    output
}

fn haversine(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let lon1 = lon1.to_radians();
    let lon2 = lon2.to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();
    let lon_delta = ((lon2 - lon1) / 2.0).sin();
    if lon_delta == 0.0 {
        return EARTH_RADIUS_METERS * (lat2 - lat1).abs();
    }
    let lat_delta = ((lat2 - lat1) / 2.0).sin();
    let a = lat_delta * lat_delta + lat1.cos() * lat2.cos() * lon_delta * lon_delta;
    2.0 * EARTH_RADIUS_METERS * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Action;

    fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn geohash_round_trip_matches_redis_reference_coordinates() {
        let (lon, lat) = decode_geohash(interleave_geohash(13.361389, 38.115556) as f64).unwrap();
        assert_eq!(format_human(lon), "13.36138933897018433");
        assert_eq!(format_human(lat), "38.11555639549629859");
        assert_eq!(standard_geohash(lon, lat), b"sqc8b49rny0".to_vec());
        let (lon, lat) = decode_geohash(interleave_geohash(15.087269, 37.502669) as f64).unwrap();
        assert_eq!(format_human(lon), "15.08726745843887329");
        assert_eq!(format_human(lat), "37.50266842333162032");
        assert_eq!(standard_geohash(lon, lat), b"sqdtr74hyu0".to_vec());
    }

    #[test]
    fn geoadd_geodist_and_search_are_consistent() {
        let store = Store::new(4096);
        assert_eq!(
            run(
                &store,
                &[b"GEOADD", b"Sicily", b"13.361389", b"38.115556", b"Palermo"]
            ),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"GEOADD", b"Sicily", b"15.087269", b"37.502669", b"Catania"]
            ),
            b":1\r\n"
        );
        assert_eq!(
            run(&store, &[b"GEODIST", b"Sicily", b"Palermo", b"Catania"]),
            b"$11\r\n166274.1516\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"GEODIST", b"Sicily", b"Palermo", b"Catania", b"km"]
            ),
            b"$8\r\n166.2742\r\n"
        );
        assert_eq!(
            run(&store, &[b"GEOPOS", b"Sicily", b"Palermo", b"missing"]),
            b"*2\r\n*2\r\n$20\r\n13.36138933897018433\r\n$20\r\n38.11555639549629859\r\n*-1\r\n"
        );
        assert_eq!(
            run(&store, &[b"GEOHASH", b"Sicily", b"Palermo", b"Catania"]),
            b"*2\r\n$11\r\nsqc8b49rny0\r\n$11\r\nsqdtr74hyu0\r\n"
        );
        let search = run(
            &store,
            &[
                b"GEOSEARCH",
                b"Sicily",
                b"FROMLONLAT",
                b"15",
                b"37",
                b"BYRADIUS",
                b"200",
                b"km",
                b"ASC",
            ],
        );
        assert!(search.starts_with(b"*2\r\n$7\r\nCatania\r\n$7\r\nPalermo\r\n"));
    }

    #[test]
    fn geoadd_options_errors_and_store_search_are_atomic() {
        let store = Store::new(4096);
        assert_eq!(
            run(
                &store,
                &[
                    b"GEOADD",
                    b"Sicily",
                    b"NX",
                    b"13.361389",
                    b"38.115556",
                    b"Palermo"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"GEOADD", b"Sicily", b"NX", b"13.5", b"38.2", b"Palermo"]
            ),
            b":0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"GEOADD", b"Sicily", b"XX", b"CH", b"13.5", b"38.2", b"Palermo"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"GEOADD", b"Sicily", b"NX", b"XX", b"0", b"0", b"x"]
            ),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            run(&store, &[b"GEOADD", b"Sicily", b"181", b"0", b"bad"]),
            b"-ERR invalid longitude,latitude pair 181.000000,0.000000\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"GEOSEARCHSTORE",
                    b"out",
                    b"Sicily",
                    b"FROMMEMBER",
                    b"Palermo",
                    b"BYRADIUS",
                    b"100",
                    b"km",
                    b"ASC"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"TYPE", b"out"]), b"+zset\r\n");
        assert_eq!(
            run(
                &store,
                &[
                    b"GEOSEARCHSTORE",
                    b"out",
                    b"Sicily",
                    b"FROMLONLAT",
                    b"0",
                    b"0",
                    b"BYRADIUS",
                    b"1",
                    b"km"
                ]
            ),
            b":0\r\n"
        );
        assert_eq!(run(&store, &[b"EXISTS", b"out"]), b":0\r\n");
        assert!(
            run(
                &store,
                &[
                    b"GEOSEARCH",
                    b"Sicily",
                    b"FROMMEMBER",
                    b"Palermo",
                    b"BYBOX",
                    b"100",
                    b"100",
                    b"km",
                    b"WITHCOORD",
                    b"WITHDIST",
                    b"WITHHASH"
                ]
            )
            .starts_with(b"*1\r\n*4\r\n$7\r\nPalermo\r\n")
        );
    }
}
