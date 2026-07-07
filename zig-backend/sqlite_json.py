#!/usr/bin/env python3
import sqlite3, json, sys

db_path = "/Users/careybalboa/Documents/GitHub/archetype-mesh-benchmark/data/archetype_mesh_benchmark.sqlite"
sql_file = sys.argv[1] if len(sys.argv) > 1 else None
sql = ""
if sql_file:
    with open(sql_file, "r") as f:
        sql = f.read().strip()
if not sql:
    print("[]")
    sys.exit(0)
con = sqlite3.connect(db_path)
cur = con.cursor()
try:
    cur.execute(sql)
    rows = cur.fetchall()
    print(rows[0][0] if rows and rows[0][0] else "[]")
except Exception as e:
    print("[]")
finally:
    con.close()
