#!/usr/bin/env python3
import argparse
import sys

RELEASE = "2026-10-21.0"
BLOCK = 2048
CELL = 0.25
CELL_COARSE = 1.0
K = 8
SEGMENTS = (
    f"s3://overturemaps-us-west-2/release/{RELEASE}/theme=transportation/type=segment/*.parquet"
)
DIVISIONS = (
    f"s3://overturemaps-us-west-2/release/{RELEASE}/theme=divisions/type=division/*.parquet"
)

PAIRS = {
    "a": "àáâãäåÀÁÂÃÄÅæÆ",
    "e": "èéêëÈÉÊË",
    "i": "ìíîïÌÍÎÏ",
    "o": "òóôõöÒÓÔÕÖøØœŒ",
    "u": "ùúûüÙÚÛÜ",
    "c": "çÇ",
    "n": "ñÑ",
    "s": "ß",
    "y": "ýÿ",
    "l": "łŁ",
}
FOLD = {ord(c): to for to, chars in PAIRS.items() for c in chars}

OFFSETS = "(-1,-1),(-1,0),(-1,1),(0,-1),(0,0),(0,1),(1,-1),(1,0),(1,1)"


def fold(s):
    return s.translate(FOLD).lower()


def fold_sql(col):
    expr = col
    for to, chars in PAIRS.items():
        for c in chars:
            expr = f"replace({expr}, '{c}', '{to}')"
    return f"lower({expr})"


def cell_x(col, cell):
    return f"CAST(floor(({col} + 180.0) / {cell}) AS BIGINT)"


def cell_y(col, cell):
    return f"CAST(floor(({col} + 90.0) / {cell}) AS BIGINT)"


def build(out, bbox, per_name):
    import os
    import shutil

    from duckdb import connect

    tmp = os.path.join(os.path.dirname(os.path.abspath(out)) or ".", ".streets-tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)

    con = connect()
    con.execute("INSTALL httpfs; LOAD httpfs;")
    con.execute("SET s3_region='us-west-2';")
    con.execute("SET memory_limit='8GB'; SET preserve_insertion_order=false;")
    con.execute(f"SET temp_directory='{tmp}'")

    seg_where = "names.primary IS NOT NULL"
    div_where = "subtype = 'locality' AND names.primary IS NOT NULL"
    ctry_where = "subtype = 'country' AND country IS NOT NULL"
    if bbox:
        x1, y1, x2, y2 = bbox
        seg_where += f" AND bbox.xmin BETWEEN {x1} AND {x2} AND bbox.ymin BETWEEN {y1} AND {y2}"
        div_where += (
            f" AND bbox.xmin BETWEEN {x1 - 1.0} AND {x2 + 1.0}"
            f" AND bbox.ymin BETWEEN {y1 - 1.0} AND {y2 + 1.0}"
        )
        ctry_where += (
            f" AND bbox.xmax >= {x1} AND bbox.xmin <= {x2}"
            f" AND bbox.ymax >= {y1} AND bbox.ymin <= {y2}"
        )

    con.execute(f"""
    CREATE TEMP TABLE places AS
    SELECT names.primary AS name,
           round(bbox.ymin, 3) AS lat,
           round(bbox.xmin, 3) AS lon,
           count(*) AS n,
           {cell_x('round(bbox.xmin, 3)', CELL)} AS cx,
           {cell_y('round(bbox.ymin, 3)', CELL)} AS cy
    FROM read_parquet('{SEGMENTS}')
    WHERE {seg_where}
    GROUP BY 1, 2, 3
    """)

    con.execute(f"""
    CREATE TEMP TABLE localities_raw AS
    SELECT names.primary AS lname,
           country AS ccode,
           (bbox.xmin + bbox.xmax) / 2 AS lon,
           (bbox.ymin + bbox.ymax) / 2 AS lat,
           COALESCE(population, 0) AS pop,
           COALESCE(cartography.prominence, 0) AS prom
    FROM read_parquet('{DIVISIONS}')
    WHERE {div_where}
    QUALIFY row_number() OVER (
      PARTITION BY names.primary, country
      ORDER BY COALESCE(population, 0) DESC, COALESCE(cartography.prominence, 0) DESC
    ) = 1
    """)

    con.execute(f"""
    CREATE TEMP TABLE localities AS
    SELECT lname, ccode, lat, lon, pop, prom, cx, cy
    FROM (
      SELECT lname, ccode, lat, lon, pop, prom,
             {cell_x('lon', CELL)} AS cx,
             {cell_y('lat', CELL)} AS cy,
             row_number() OVER (
               PARTITION BY {cell_x('lon', CELL)}, {cell_y('lat', CELL)}
               ORDER BY pop DESC, prom DESC
             ) AS rn
      FROM localities_raw
    ) WHERE rn <= {K}
    """)

    con.execute(f"""
    CREATE TEMP TABLE joined AS
    SELECT name, lat, lon, n,
           best.lname AS lname, best.ccode AS ccode, best.pop AS pop, best.prom AS prom
    FROM (
      SELECT p.name, p.lat, p.lon, p.n,
             arg_min(
               struct_pack(lname := l.lname, ccode := l.ccode, prom := l.prom, pop := l.pop),
               ((p.lat - l.lat) * (p.lat - l.lat))
                 + ((p.lon - l.lon) * cos(radians(p.lat)))
                   * ((p.lon - l.lon) * cos(radians(p.lat)))
             ) AS best
      FROM places p
      JOIN (
        SELECT lname, ccode, lat, lon, pop, prom,
               cx + off.dx AS cell_x, cy + off.dy AS cell_y
        FROM localities
        CROSS JOIN (VALUES {OFFSETS}) AS off(dx, dy)
      ) l ON l.cell_x = p.cx AND l.cell_y = p.cy
      GROUP BY p.name, p.lat, p.lon, p.n
    )
    """)

    con.execute("""
    CREATE TEMP TABLE placed AS
    SELECT p.name, p.lat, p.lon, p.n, j.lname, j.ccode, j.pop, j.prom
    FROM places p
    LEFT JOIN joined j
      ON p.name = j.name AND p.lat = j.lat AND p.lon = j.lon
    """)

    con.execute(f"""
    CREATE TEMP TABLE localities_coarse AS
    SELECT lname, ccode, lat, lon, pop, prom, cx, cy
    FROM (
      SELECT lname, ccode, lat, lon, pop, prom,
             {cell_x('lon', CELL_COARSE)} AS cx,
             {cell_y('lat', CELL_COARSE)} AS cy,
             row_number() OVER (
               PARTITION BY {cell_x('lon', CELL_COARSE)}, {cell_y('lat', CELL_COARSE)}
               ORDER BY pop DESC, prom DESC
             ) AS rn
      FROM localities_raw
    ) WHERE rn <= {K}
    """)

    con.execute(f"""
    CREATE TEMP TABLE placed_null AS
    SELECT name, lat, lon, n,
           {cell_x('lon', CELL_COARSE)} AS cx,
           {cell_y('lat', CELL_COARSE)} AS cy
    FROM placed WHERE lname IS NULL
    """)

    con.execute(f"""
    CREATE TEMP TABLE joined_coarse AS
    SELECT name, lat, lon,
           best.lname AS lname, best.ccode AS ccode, best.pop AS pop, best.prom AS prom
    FROM (
      SELECT p.name, p.lat, p.lon,
             arg_min(
               struct_pack(lname := l.lname, ccode := l.ccode, prom := l.prom, pop := l.pop),
               ((p.lat - l.lat) * (p.lat - l.lat))
                 + ((p.lon - l.lon) * cos(radians(p.lat)))
                   * ((p.lon - l.lon) * cos(radians(p.lat)))
             ) AS best
      FROM placed_null p
      JOIN (
        SELECT lname, ccode, lat, lon, pop, prom,
               cx + off.dx AS cell_x, cy + off.dy AS cell_y
        FROM localities_coarse
        CROSS JOIN (VALUES {OFFSETS}) AS off(dx, dy)
      ) l ON l.cell_x = p.cx AND l.cell_y = p.cy
      GROUP BY p.name, p.lat, p.lon
    )
    """)

    con.execute("""
    UPDATE placed SET lname = j.lname, ccode = j.ccode, pop = j.pop, prom = j.prom
    FROM joined_coarse j
    WHERE placed.lname IS NULL
      AND placed.name = j.name AND placed.lat = j.lat AND placed.lon = j.lon
    """)

    con.execute(f"""
    CREATE TEMP TABLE countries AS
    SELECT country AS ccode, bbox.xmin, bbox.xmax, bbox.ymin, bbox.ymax
    FROM read_parquet('{DIVISIONS}')
    WHERE {ctry_where}
    QUALIFY row_number() OVER (PARTITION BY country ORDER BY bbox.ymax DESC) = 1
    """)

    con.execute("""
    UPDATE placed SET ccode = c.ccode
    FROM countries c
    WHERE placed.lname IS NULL
      AND placed.lon >= c.xmin AND placed.lon <= c.xmax
      AND placed.lat >= c.ymin AND placed.lat <= c.ymax
    """)

    sql = f"""
    WITH labeled AS (
      SELECT name,
             {fold_sql('name')} AS folded,
             CASE
               WHEN lname IS NOT NULL AND ccode IS NOT NULL THEN lname || ', ' || ccode
               WHEN lname IS NOT NULL THEN lname
               ELSE ccode
             END AS place,
             COALESCE(prom, 0) AS prom,
             COALESCE(pop, 0) AS pop,
             lat, lon, n
      FROM placed
    ),
    collapsed AS (
      SELECT min(name) AS name,
             folded,
             place,
             round(AVG(lat), 3) AS lat,
             round(AVG(lon), 3) AS lon,
             MAX(prom) AS prom,
             MAX(pop) AS pop,
             SUM(n) AS n
      FROM labeled
      GROUP BY folded, place
    ),
    kept AS (
      SELECT name, folded, place, lat, lon, prom, pop,
             row_number() OVER (
               PARTITION BY folded
               ORDER BY pop DESC, prom DESC, n DESC, lat, lon
             ) AS rank
      FROM collapsed
    )
    SELECT name, folded, lat, lon, place
    FROM kept
    WHERE rank <= {per_name}
    ORDER BY folded, name, pop DESC, prom DESC, lat, lon
    """
    cur = con.execute(sql)
    written = 0
    names = set()
    sparse = []
    with open(out, "w+", encoding="utf-8") as fh:
        fh.write(" " * 63 + "\n")
        while True:
            batch = cur.fetchmany(20000)
            if not batch:
                break
            for name, key, lat, lon, place in batch:
                if written % BLOCK == 0:
                    sparse.append((key, fh.tell()))
                if place:
                    fh.write(f"{key}\t{name}\t{lat}\t{lon}\t{place}\n")
                else:
                    fh.write(f"{key}\t{name}\t{lat}\t{lon}\n")
                names.add(name)
                written += 1
            if written % 200000 < 20000:
                print(f"  {written} places", file=sys.stderr)
        index_at = fh.tell()
        for key, offset in sparse:
            fh.write(f"{key}\t{offset}\n")
        head = f"alexandria-streets 4 {index_at} {written}"
        fh.seek(0)
        fh.write(head.ljust(63) + "\n")
    expect = (written + BLOCK - 1) // BLOCK
    if len(sparse) != expect:
        raise SystemExit(f"block index is wrong: {len(sparse)} entries for {written} rows")
    shutil.rmtree(tmp, ignore_errors=True)
    print(f"{written} places, {len(names)} names, {len(sparse)} blocks -> {out}", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--bbox", nargs=4, type=float, metavar=("X1", "Y1", "X2", "Y2"))
    ap.add_argument("--per-name", type=int, default=6)
    a = ap.parse_args()
    build(a.out, a.bbox, a.per_name)


if __name__ == "__main__":
    main()
