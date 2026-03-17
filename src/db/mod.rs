use anyhow::Result;
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct PerfRow {
    pub total: i64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: f64,
    pub avg_mult: f64,
    pub median_mult: f64,
}

impl Db {
    pub fn bot_perf_lifetime(&mut self) -> Result<PerfRow> {
        self.bot_perf_window(None)
    }

    pub fn bot_perf_last_n(&mut self, n: i64) -> Result<PerfRow> {
        self.bot_perf_window(Some(n))
    }

    fn bot_perf_window(&mut self, last_n: Option<i64>) -> Result<PerfRow> {
        let limit_clause = if last_n.is_some() { "LIMIT ?1" } else { "" };

        let sql = format!(
            r#"
        WITH graded AS (
          SELECT
            o.result AS result,
            c.fdv_usd AS call_fdv,
            o.fdv_usd AS peak_fdv,
            o.call_ts AS call_ts
          FROM call_outcomes o
          JOIN calls c
            ON c.mint = o.mint AND c.ts = o.call_ts
          ORDER BY o.call_ts DESC
          {limit_clause}
        )
        SELECT
          COUNT(*) AS total,
          SUM(CASE WHEN result='win'  THEN 1 ELSE 0 END) AS wins,
          SUM(CASE WHEN result='loss' THEN 1 ELSE 0 END) AS losses,
          AVG(CASE WHEN call_fdv > 0 THEN (peak_fdv / call_fdv) ELSE NULL END) AS avg_mult
        FROM graded
        "#,
            limit_clause = limit_clause
        );

        let (total, wins, losses, avg_mult): (i64, i64, i64, Option<f64>) = match last_n {
            Some(n) => self.conn.query_row(&sql, params![n], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?,
            None => self.conn.query_row(&sql, [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?,
        };

        let win_rate = if total > 0 {
            (wins as f64) / (total as f64)
        } else {
            0.0
        };

        Ok(PerfRow {
            total,
            wins,
            losses,
            win_rate,
            avg_mult: avg_mult.unwrap_or(0.0),
            median_mult: 0.0,
        })
    }

    pub fn open(path: &str) -> Result<Self> {
        // --- DEBUG: print the db path we are actually using ---
        let abs: PathBuf = std::fs::canonicalize(path)
            .unwrap_or_else(|_| std::env::current_dir().unwrap().join(path));

        eprintln!("🗄️ Db::open path(arg) = {}", path);
        eprintln!("🗄️ Db::open path(abs) = {}", abs.display());

        let _create_new = !Path::new(path).exists();
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // Apply schema (idempotent)
        let schema = include_str!("schema.sql");
        conn.execute_batch(schema)?;

        // ---- calls.tag migration (safe) ----
        // Adds calls.tag if it doesn't exist (older DBs)
        let has_calls_tag = {
            let mut has = false;
            let mut stmt = conn.prepare("PRAGMA table_info(calls)")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(1)?; // column name
                if name == "tag" {
                    has = true;
                    break;
                }
            }
            has
        };

        if !has_calls_tag {
            conn.execute_batch("ALTER TABLE calls ADD COLUMN tag TEXT NOT NULL DEFAULT '';")?;
        }

        // -------------------------
        // wallet_scoring_state safety + migration
        // -------------------------
        // Ensure table exists (in case schema.sql is older)
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS wallet_scoring_state (
              id             INTEGER PRIMARY KEY CHECK (id = 1),
              last_scored_ts INTEGER NOT NULL DEFAULT 0
            );
            INSERT OR IGNORE INTO wallet_scoring_state (id, last_scored_ts) VALUES (1, 0);
            "#,
        )?;

        // Detect column drift: some DBs may have last_ts instead of last_scored_ts
        let (has_last_scored_ts, has_last_ts) = {
            let mut has_scored = false;
            let mut has_last = false;
            let mut stmt = conn.prepare("PRAGMA table_info(wallet_scoring_state)")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(1)?; // column name
                if name == "last_scored_ts" {
                    has_scored = true;
                } else if name == "last_ts" {
                    has_last = true;
                }
            }
            (has_scored, has_last)
        };

        // If last_scored_ts is missing, add it.
        if !has_last_scored_ts {
            conn.execute_batch(
                "ALTER TABLE wallet_scoring_state ADD COLUMN last_scored_ts INTEGER NOT NULL DEFAULT 0;",
            )?;
            // Ensure row exists for id=1 (older DBs might lack it)
            let _ = conn.execute(
                "INSERT OR IGNORE INTO wallet_scoring_state (id, last_scored_ts) VALUES (1, 0)",
                [],
            )?;
        }

        // Backfill last_scored_ts from last_ts if present and last_scored_ts is 0
        if has_last_ts {
            conn.execute_batch(
                r#"
                UPDATE wallet_scoring_state
                SET last_scored_ts = CASE
                  WHEN last_scored_ts IS NULL OR last_scored_ts = 0 THEN COALESCE(last_ts, 0)
                  ELSE last_scored_ts
                END
                WHERE id = 1;
                "#,
            )?;
        }

        // Mayhem Mode migration (safe, idempotent) - add is_mayhem column to mint_snapshots
        let has_is_mayhem = {
            let mut has = false;
            let mut stmt = conn.prepare("PRAGMA table_info(mint_snapshots)")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(1)?; // column name
                if name == "is_mayhem" {
                    has = true;
                    break;
                }
            }
            has
        };

        if !has_is_mayhem {
            conn.execute_batch(
                "ALTER TABLE mint_snapshots ADD COLUMN is_mayhem INTEGER NOT NULL DEFAULT 0;",
            )?;
            eprintln!("🗄️ Added is_mayhem column to mint_snapshots (migration applied)");
        }

        Ok(Self { conn })
    }

    pub fn fdv_change_pct(&mut self, mint: &str, now_ts: i64, lookback_secs: i64) -> Option<f64> {
        let since = now_ts - lookback_secs;

        let mut stmt = self
            .conn
            .prepare(
                r#"
        SELECT fdv_usd
        FROM mint_snapshots
        WHERE mint = ?1
          AND ts >= ?2
          AND fdv_usd IS NOT NULL
        ORDER BY ts ASC
        LIMIT 1
        "#,
            )
            .ok()?;

        let old_fdv: f64 = stmt.query_row(params![mint, since], |r| r.get(0)).ok()?;

        let mut stmt2 = self
            .conn
            .prepare(
                r#"
        SELECT fdv_usd
        FROM mint_snapshots
        WHERE mint = ?1
          AND fdv_usd IS NOT NULL
        ORDER BY ts DESC
        LIMIT 1
        "#,
            )
            .ok()?;

        let new_fdv: f64 = stmt2.query_row(params![mint], |r| r.get(0)).ok()?;

        if old_fdv <= 0.0 {
            return None;
        }

        Some((new_fdv - old_fdv) / old_fdv)
    }

    // =========================
    // Wallet learning writes
    // =========================

    /// Ensure a wallet exists in `wallets` and update last_seen_ts.
    pub fn touch_wallet(&mut self, wallet: &str, ts: i64) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO wallets (wallet, score, last_seen_ts, notes)
            VALUES (?1, 0, ?2, NULL)
            ON CONFLICT(wallet) DO UPDATE SET
              last_seen_ts = CASE
                WHEN wallets.last_seen_ts IS NULL OR wallets.last_seen_ts < excluded.last_seen_ts
                THEN excluded.last_seen_ts
                ELSE wallets.last_seen_ts
              END
            "#,
            params![wallet, ts],
        )?;
        Ok(())
    }

    /// Add an observation edge (src -> dst) to `wallet_edges`.
    pub fn insert_wallet_edge(
        &mut self,
        ts: i64,
        src_wallet: &str,
        dst_wallet: Option<&str>,
        mint: Option<&str>,
        action: &str,
        sol: Option<f64>,
        sig: Option<&str>,
    ) -> Result<()> {
        // keep wallet table warm
        self.touch_wallet(src_wallet, ts)?;
        if let Some(dst) = dst_wallet {
            self.touch_wallet(dst, ts)?;
        }

        self.conn.execute(
            r#"
            INSERT INTO wallet_edges (ts, src_wallet, dst_wallet, mint, action, sol, sig)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![ts, src_wallet, dst_wallet, mint, action, sol, sig],
        )?;
        Ok(())
    }

    pub fn insert_call_outcome(
        &mut self,
        mint: &str,
        call_ts: i64,
        outcome_ts: i64,
        fdv_usd: f64,
        result: &str,
    ) -> Result<usize> {
        let n = self.conn.execute(
            r#"
        INSERT OR REPLACE INTO call_outcomes (mint, call_ts, outcome_ts, fdv_usd, result)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
            rusqlite::params![mint, call_ts, outcome_ts, fdv_usd, result],
        )?;
        Ok(n as usize)
    }

    pub fn snapshot_fdv_at_or_before(
        &mut self,
        mint: &str,
        target_ts: i64,
        min_ts: i64,
    ) -> rusqlite::Result<Option<f64>> {
        let mut stmt = self.conn.prepare(
            "SELECT fdv_usd
         FROM mint_snapshots
         WHERE mint = ?1
           AND ts <= ?2
           AND ts >= ?3
           AND fdv_usd IS NOT NULL
         ORDER BY ts DESC
         LIMIT 1",
        )?;

        let mut rows = stmt.query((mint, target_ts, min_ts))?;

        if let Some(row) = rows.next()? {
            Ok(Some(row.get::<_, f64>(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn peak_fdv_for_mint_window(
        &mut self,
        mint: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Option<f64>> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT MAX(fdv_usd)
        FROM mint_snapshots
        WHERE mint = ?1
          AND ts BETWEEN ?2 AND ?3
          AND fdv_usd IS NOT NULL
        "#,
        )?;

        let out: Option<f64> = stmt.query_row(params![mint, start_ts, end_ts], |r| r.get(0))?;
        Ok(out)
    }

    pub fn insert_skip_debug(
        &mut self,
        ts: i64,
        mint: &str,
        reason: &str,
        bucket: &str,
        fdv_usd: Option<f64>,
    ) -> anyhow::Result<()> {
        use rusqlite::params;

        self.conn.execute(
            r#"
        INSERT INTO skip_debug(ts, mint, reason, bucket, fdv_usd)
        VALUES(?1, ?2, ?3, ?4, ?5)
        "#,
            params![ts, mint, reason, bucket, fdv_usd],
        )?;

        Ok(())
    }

    pub fn calls_missing_outcomes(
        &mut self,
        older_than_ts: i64,
        limit: i64,
    ) -> Result<Vec<(String, i64, f64)>> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT c.mint, c.ts, c.fdv_usd
        FROM calls c
        LEFT JOIN call_outcomes o
          ON o.mint = c.mint AND o.call_ts = c.ts
        WHERE o.mint IS NULL
          AND c.ts <= ?1
        ORDER BY c.ts ASC
        LIMIT ?2
        "#,
        )?;

        let mut rows = stmt.query(params![older_than_ts, limit])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?, r.get(2)?));
        }
        Ok(out)
    }

    pub fn mint_first_seen_ts(&mut self, mint: &str) -> rusqlite::Result<Option<i64>> {
        let sql = r#"
        SELECT MIN(ts)
        FROM mint_snapshots
        WHERE mint = ?1
    "#;

        self.conn.query_row(sql, [mint], |r| r.get(0)).optional()
    }

    pub fn mint_last_seen_ts(&mut self, mint: &str) -> rusqlite::Result<Option<i64>> {
        let sql = r#"
        SELECT MAX(ts)
        FROM mint_snapshots
        WHERE mint = ?1
    "#;

        self.conn.query_row(sql, [mint], |r| r.get(0)).optional()
    }

    pub fn mint_max_gap_secs_recent(
        &mut self,
        mint: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> rusqlite::Result<Option<i64>> {
        let sql = r#"
        SELECT MAX(ts - prev_ts) AS max_gap
        FROM (
            SELECT
                ts,
                LAG(ts) OVER (ORDER BY ts) AS prev_ts
            FROM mint_snapshots
            WHERE mint = ?1
              AND ts BETWEEN ?2 AND ?3
        )
        WHERE prev_ts IS NOT NULL
    "#;

        self.conn
            .query_row(sql, (mint, start_ts, end_ts), |r| r.get(0))
            .optional()
    }

    pub fn wallet_outcomes_last_call_ts(&mut self) -> Result<i64> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT COALESCE(MAX(call_ts), 0)
        FROM wallet_outcomes
        "#,
        )?;
        let v: i64 = stmt.query_row([], |r| r.get(0))?;
        Ok(v)
    }

    // --- Watchlist wallets (dynamic helius wallet-learning) ---

    pub fn get_watchlist_wallets(&mut self, limit: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();

        let mut stmt = match self.conn.prepare(
            r#"
        SELECT wallet
        FROM watchlist_wallets
        ORDER BY last_seen_ts DESC NULLS LAST
        LIMIT ?1
        "#,
        ) {
            Ok(s) => s,
            Err(_) => return out,
        };

        let rows = match stmt.query_map([limit as i64], |r| r.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return out,
        };

        for w in rows.flatten() {
            let w = w.trim();
            if !w.is_empty() {
                out.push(w.to_string());
            }
        }

        out
    }

    pub fn upsert_watchlist_wallet(
        &mut self,
        wallet: &str,
        seen_ts: i64,
        tags: &str,
    ) -> anyhow::Result<bool> {
        use rusqlite::params;

        let wallet = wallet.trim();
        if wallet.is_empty() {
            return Ok(false);
        }

        // Normalize incoming tags into distinct tokens
        // Example input: "auto,scored" -> ["auto", "scored"]
        let mut tokens: Vec<String> = tags
            .split(',')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect();

        // De-dupe input tokens while preserving order
        {
            let mut seen = std::collections::HashSet::<String>::new();
            tokens.retain(|t| seen.insert(t.clone()));
        }

        // 1) Ensure row exists; do NOT overwrite tags here (keep whatever is stored)
        //    last_seen_ts only moves forward
        let inserted = self.conn.execute(
            r#"
        INSERT INTO watchlist_wallets (wallet, first_seen_ts, last_seen_ts, tags, wins, losses)
        VALUES (?1, ?2, ?3, '', 0, 0)
        ON CONFLICT(wallet) DO UPDATE SET
          last_seen_ts = CASE
            WHEN watchlist_wallets.last_seen_ts IS NULL THEN excluded.last_seen_ts
            WHEN excluded.last_seen_ts > watchlist_wallets.last_seen_ts THEN excluded.last_seen_ts
            ELSE watchlist_wallets.last_seen_ts
          END
        "#,
            params![wallet, seen_ts, seen_ts],
        )?;

        // 2) Merge tags token-by-token (no duplicates)
        //    This handles both single-token and multi-token inputs safely.
        for tok in tokens {
            self.conn.execute(
                r#"
            UPDATE watchlist_wallets
            SET tags = CASE
              WHEN tags IS NULL OR tags = '' THEN ?2
              WHEN instr(',' || tags || ',', ',' || ?2 || ',') > 0 THEN tags
              ELSE tags || ',' || ?2
            END
            WHERE wallet = ?1
            "#,
                params![wallet, tok],
            )?;
        }

        Ok(inserted == 1)
    }

    // --- Watchlist promotion / rotation ---
    pub fn promote_watchlist_from_scored_wallets(
        &mut self,
        now_ts: i64,
        promote_n: usize,
        min_score: i64,
        max_watchlist: usize,
    ) -> anyhow::Result<(usize, usize)> {
        // candidates: Vec<(wallet, seen_ts)>
        let mut candidates: Vec<(String, i64)> = Vec::new();

        {
            let mut stmt = self.conn.prepare(
                r#"
            SELECT wallet, COALESCE(last_seen_ts, 0) AS last_seen_ts
            FROM wallets
            WHERE score >= ?1
              AND last_seen_ts IS NOT NULL
              AND last_seen_ts >= (?2 - 86400)
            ORDER BY score DESC, last_seen_ts DESC, RANDOM()
            LIMIT ?3
            "#,
            )?;

            let rows = stmt.query_map(
                rusqlite::params![min_score, now_ts, promote_n as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )?;

            for row in rows.flatten() {
                let (w, seen_ts) = row;
                let w = w.trim().to_string();
                if !w.is_empty() {
                    candidates.push((w, seen_ts));
                }
            }
        }

        let mut added = 0usize;

        for (wallet, seen_ts) in candidates.iter() {
            // insert-if-missing (first_seen_ts=now, last_seen_ts=seen_ts)
            let inserted = self.conn.execute(
                r#"
            INSERT OR IGNORE INTO watchlist_wallets
              (wallet, first_seen_ts, last_seen_ts, tags, wins, losses)
            VALUES
              (?1, ?2, ?3, ?4, 0, 0)
            "#,
                rusqlite::params![wallet, now_ts, *seen_ts, "auto,scored"],
            )?;

            if inserted > 0 {
                added += 1;
            }

            // if already exists, do NOT stomp last_seen_ts to now
            // only advance it to seen_ts if newer; merge tags
            self.conn.execute(
                r#"
            UPDATE watchlist_wallets
            SET last_seen_ts = CASE
                  WHEN last_seen_ts IS NULL THEN ?2
                  WHEN ?2 > last_seen_ts THEN ?2
                  ELSE last_seen_ts
                END,
                tags = CASE
                  WHEN tags IS NULL OR tags = '' THEN ?3
                  WHEN instr(',' || tags || ',', ',' || ?3 || ',') > 0 THEN tags
                  ELSE tags || ',' || ?3
                END
            WHERE wallet = ?1
            "#,
                rusqlite::params![wallet, *seen_ts, "auto,scored"],
            )?;
        }

        let pruned = self.prune_watchlist_wallets(max_watchlist)?;
        Ok((added, pruned))
    } // ✅ THIS BRACE WAS MISSING IN YOUR FILE

    pub fn prune_watchlist_wallets(&mut self, keep_n: usize) -> anyhow::Result<usize> {
        let rows = self.conn.execute(
            r#"
        DELETE FROM watchlist_wallets
        WHERE wallet IN (
          SELECT wallet FROM watchlist_wallets
          ORDER BY last_seen_ts DESC NULLS LAST
          LIMIT -1 OFFSET ?1
        )
        "#,
            rusqlite::params![keep_n as i64],
        )?;

        Ok(rows as usize)
    }

    pub fn watchlist_top_debug(
        &mut self,
        limit: usize,
    ) -> rusqlite::Result<Vec<(String, i64, i64, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT wl.wallet,
               COALESCE(w.score, 0) AS score,
               wl.wins,
               wl.losses,
               COALESCE(wl.last_seen_ts, 0) AS last_seen_ts
        FROM watchlist_wallets wl
        LEFT JOIN wallets w ON w.wallet = wl.wallet
        ORDER BY score DESC, wl.wins DESC, last_seen_ts DESC
        LIMIT ?1
        "#,
        )?;

        let rows = stmt.query_map([limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn bump_watchlist_wl_for_wallets(
        &mut self,
        wallets: &[String],
        is_win: bool,
    ) -> anyhow::Result<usize> {
        let col = if is_win { "wins" } else { "losses" };

        let tx = self.conn.transaction()?;
        let mut total: usize = 0;

        {
            let sql = format!(
                r#"
            UPDATE watchlist_wallets
            SET {col} = {col} + 1
            WHERE wallet = ?1
            "#
            );

            let mut stmt = tx.prepare(&sql)?;

            for w in wallets {
                // only updates if wallet exists in watchlist_wallets
                total += stmt.execute(rusqlite::params![w])? as usize;
            }
        } // ✅ stmt dropped here BEFORE commit

        tx.commit()?;
        Ok(total)
    }
    /// Pull graded call outcomes newer than `after_call_ts`.
    /// Returns: Vec<(mint, call_ts, outcome_ts, peak_fdv, result, call_fdv)>
    pub fn graded_calls_since(
        &mut self,
        after_call_ts: i64,
        limit: i64,
    ) -> Result<Vec<(String, i64, i64, f64, String, f64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              o.mint,
              o.call_ts,
              o.outcome_ts,
              o.fdv_usd        AS peak_fdv,
              o.result,
              c.fdv_usd        AS call_fdv
            FROM call_outcomes o
            JOIN calls c
              ON c.mint = o.mint AND c.ts = o.call_ts
            WHERE o.call_ts > ?1
            ORDER BY o.call_ts ASC
            LIMIT ?2
            "#,
        )?;

        let mut rows = stmt.query(params![after_call_ts, limit])?;
        let mut out = Vec::new();

        while let Some(r) = rows.next()? {
            out.push((
                r.get(0)?, // mint
                r.get(1)?, // call_ts
                r.get(2)?, // outcome_ts
                r.get(3)?, // peak_fdv
                r.get(4)?, // result
                r.get(5)?, // call_fdv
            ));
        }

        Ok(out)
    }

    pub fn insert_wallet_outcome(
        &mut self,
        wallet: &str,
        mint: &str,
        call_ts: i64,
        outcome_ts: i64,
        result: &str,
        call_fdv: f64,
        peak_fdv: f64,
        edges: i64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
        INSERT OR REPLACE INTO wallet_outcomes
          (wallet, mint, call_ts, outcome_ts, result, call_fdv, peak_fdv, edges)
        VALUES
          (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
            params![wallet, mint, call_ts, outcome_ts, result, call_fdv, peak_fdv, edges],
        )?;
        Ok(())
    }

    pub fn wallets_for_call(
        &mut self,
        mint: &str,
        call_ts: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<(String, i64)>> {
        use rusqlite::params;

        let mut stmt = self.conn.prepare(
            r#"
        SELECT wallet, edges
        FROM call_top_wallets
        WHERE mint = ?1 AND call_ts = ?2
        ORDER BY edges DESC
        LIMIT ?3
        "#,
        )?;

        let mut rows = stmt.query(params![mint, call_ts, limit])?;
        let mut out = Vec::new();

        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }

        Ok(out)
    }

    pub fn call_top_wallets_for_call(
        &mut self,
        mint: &str,
        call_ts: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<String>> {
        Ok(self
            .wallets_for_call(mint, call_ts, limit)?
            .into_iter()
            .map(|(w, _edges)| w)
            .collect())
    }

    // =========================
    // Wallet reputation reads/writes
    // =========================

    pub fn watchlist_is_whale(&mut self, wallet: &str) -> anyhow::Result<bool> {
        use rusqlite::params;
        let mut stmt = self.conn.prepare(
            r#"SELECT 1 FROM watchlist_wallets WHERE wallet=?1 AND tags LIKE '%WHALE%' LIMIT 1"#,
        )?;
        let mut rows = stmt.query(params![wallet])?;
        Ok(rows.next()?.is_some())
    }

    pub fn wallet_score(&mut self, wallet: &str) -> anyhow::Result<i64> {
        use rusqlite::params;
        let mut stmt = self
            .conn
            .prepare(r#"SELECT COALESCE(score,0) FROM wallets WHERE wallet=?1"#)?;
        let v: i64 = stmt.query_row(params![wallet], |r| r.get(0)).unwrap_or(0);
        Ok(v)
    }

    // =========================
    // Call-time wallet fingerprints
    // =========================

    /// Top wallets by *edge count* for a mint in a recent time window.
    /// This is our best proxy right now for "top holders / most involved wallets"
    /// until we add real token-balance holders via RPC.
    pub fn top_wallets_by_edges_recent(
        &mut self,
        mint: &str,
        now_ts: i64,
        lookback_secs: i64,
        limit: i64,
    ) -> Result<Vec<(String, i64)>> {
        let since = now_ts - lookback_secs.max(1);

        let mut stmt = self.conn.prepare(
            r#"
        SELECT src_wallet, COUNT(*) AS edges
        FROM wallet_edges
        WHERE mint = ?1 AND ts >= ?2 AND ts <= ?3
        GROUP BY src_wallet
        ORDER BY edges DESC
        LIMIT ?4
        "#,
        )?;

        let mut rows = stmt.query(params![mint, since, now_ts, limit])?;
        let mut out: Vec<(String, i64)> = Vec::new();

        while let Some(r) = rows.next()? {
            let w: String = r.get(0)?;
            let edges: i64 = r.get(1)?;
            out.push((w, edges));
        }

        Ok(out)
    }

    pub fn total_edges_for_mint_window(
        &mut self,
        mint: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<i64> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT COUNT(*) 
        FROM wallet_edges
        WHERE mint = ?1
          AND ts BETWEEN ?2 AND ?3
        "#,
        )?;
        let n: i64 = stmt.query_row(params![mint, start_ts, end_ts], |r| r.get(0))?;
        Ok(n)
    }

    /// "Early cluster" wallets: wallets that appear right near mint’s first activity.
    /// Captures degens who are consistently early even if they don't become top-edges later.
    pub fn early_wallets_for_mint(
        &mut self,
        mint: &str,
        early_window_secs: i64,
        limit: i64,
    ) -> Result<Vec<(String, i64, i64)>> {
        // find first edge ts for this mint
        let mut stmt0 = self.conn.prepare(
            r#"
        SELECT COALESCE(MIN(ts), 0)
        FROM wallet_edges
        WHERE mint = ?1
        "#,
        )?;
        let first_ts: i64 = stmt0.query_row(params![mint], |r| r.get(0)).unwrap_or(0);
        if first_ts <= 0 {
            return Ok(vec![]);
        }

        let cutoff = first_ts + early_window_secs.max(1);

        let mut stmt = self.conn.prepare(
            r#"
        SELECT
          src_wallet,
          COUNT(*) AS edges,
          MIN(ts) AS first_seen_ts
        FROM wallet_edges
        WHERE mint = ?1 AND ts >= ?2 AND ts <= ?3
        GROUP BY src_wallet
        ORDER BY first_seen_ts ASC, edges DESC
        LIMIT ?4
        "#,
        )?;

        let mut rows = stmt.query(params![mint, first_ts, cutoff, limit])?;
        let mut out: Vec<(String, i64, i64)> = Vec::new();

        while let Some(r) = rows.next()? {
            let w: String = r.get(0)?;
            let edges: i64 = r.get(1)?;
            let seen: i64 = r.get(2)?;
            out.push((w, edges, seen));
        }

        Ok(out)
    }

    pub fn insert_call_top_wallet(
        &mut self,
        mint: &str,
        call_ts: i64,
        wallet: &str,
        edges: i64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
        INSERT OR REPLACE INTO call_top_wallets (mint, call_ts, wallet, edges)
        VALUES (?1, ?2, ?3, ?4)
        "#,
            params![mint, call_ts, wallet, edges],
        )?;
        Ok(())
    }

    pub fn insert_call_top_wallets(
        &mut self,
        mint: &str,
        call_ts: i64,
        top: &[(String, i64)], // (wallet, edges)
    ) -> anyhow::Result<usize> {
        let tx = self.conn.transaction()?;
        let mut total = 0usize;

        {
            let mut stmt = tx.prepare(
                r#"
            INSERT OR REPLACE INTO call_top_wallets (mint, call_ts, wallet, edges)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            )?;

            for (w, edges) in top {
                total += stmt.execute(rusqlite::params![mint, call_ts, w, *edges])? as usize;
            }
        } // stmt dropped before commit

        tx.commit()?;
        Ok(total)
    }

    pub fn insert_call_wallet_fingerprint(
        &mut self,
        mint: &str,
        call_ts: i64,
        wallet: &str,
        reason: &str,
        metric: f64,
    ) -> Result<()> {
        self.conn.execute(
            r#"
        INSERT OR REPLACE INTO call_wallet_fingerprints (mint, call_ts, wallet, reason, metric)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
            params![mint, call_ts, wallet, reason, metric],
        )?;
        Ok(())
    }

    /// Top wallets for a mint by edge count (last N minutes).
    /// Returns Vec<(wallet, edges)>
    pub fn top_wallets_for_mint_window(
        &mut self,
        mint: &str,
        start_ts: i64,
        end_ts: i64,
        limit: i64,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
        SELECT src_wallet, COUNT(*) AS edges
        FROM wallet_edges
        WHERE mint = ?1
          AND ts BETWEEN ?2 AND ?3
        GROUP BY src_wallet
        ORDER BY edges DESC
        LIMIT ?4
        "#,
        )?;

        let mut rows = stmt.query(params![mint, start_ts, end_ts, limit])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }
        Ok(out)
    }

    /// Upsert into watchlist_wallets and merge tags.
    /// Keeps a growing list you can export to Axiom later.
    pub fn watchlist_touch_with_tag(&mut self, wallet: &str, ts: i64, tag: &str) -> Result<()> {
        // ensure row exists
        self.conn.execute(
        r#"
        INSERT INTO watchlist_wallets (wallet, first_seen_ts, last_seen_ts, tags, wins, losses)
        VALUES (?1, ?2, ?2, ?3, 0, 0)
        ON CONFLICT(wallet) DO UPDATE SET
          last_seen_ts = CASE
            WHEN watchlist_wallets.last_seen_ts IS NULL OR watchlist_wallets.last_seen_ts < excluded.last_seen_ts
            THEN excluded.last_seen_ts
            ELSE watchlist_wallets.last_seen_ts
          END,
          tags = CASE
            WHEN watchlist_wallets.tags IS NULL OR watchlist_wallets.tags = '' THEN excluded.tags
            WHEN instr(',' || watchlist_wallets.tags || ',', ',' || excluded.tags || ',') > 0 THEN watchlist_wallets.tags
            ELSE watchlist_wallets.tags || ',' || excluded.tags
          END
        "#,
        params![wallet, ts, tag],
    )?;
        Ok(())
    }

    pub fn fdv_delta_recent(&self, mint: &str, now_ts: i64, window_secs: i64) -> Result<f64> {
        let since = now_ts - window_secs.max(1);

        let mut stmt = self.conn.prepare(
        r#"
        SELECT
          (SELECT fdv_usd FROM mint_snapshots WHERE mint=?1 AND ts<=?2 AND fdv_usd IS NOT NULL ORDER BY ts DESC LIMIT 1) AS fdv_now,
          (SELECT fdv_usd FROM mint_snapshots WHERE mint=?1 AND ts>=?3 AND fdv_usd IS NOT NULL ORDER BY ts ASC  LIMIT 1) AS fdv_then
        "#,
    )?;

        let (fdv_now, fdv_then): (Option<f64>, Option<f64>) =
            stmt.query_row(params![mint, now_ts, since], |r| Ok((r.get(0)?, r.get(1)?)))?;

        Ok(fdv_now.unwrap_or(0.0) - fdv_then.unwrap_or(0.0))
    }

    /// Helpful for printing "known wallets" when you call something.
    pub fn watchlist_has_wallet(&mut self, wallet: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(r#"SELECT 1 FROM watchlist_wallets WHERE wallet=?1 LIMIT 1"#)?;
        let mut rows = stmt.query(params![wallet])?;
        Ok(rows.next()?.is_some())
    }

    /// Canonical last-scored timestamp for wallet scoring.
    /// We store it in wallet_scoring_state.last_scored_ts.
    pub fn wallet_scoring_last_ts(&mut self) -> Result<i64> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT COALESCE(last_scored_ts, 0)
            FROM wallet_scoring_state
            WHERE id = 1
            "#,
        )?;
        let v: i64 = stmt.query_row([], |r| r.get(0)).unwrap_or(0);
        Ok(v)
    }

    /// Set last scored ts. If your DB also has `last_ts`, we update it too (if exists).
    pub fn wallet_scoring_set_last_ts(&mut self, ts: i64) -> Result<()> {
        // Always update canonical column
        self.conn.execute(
            r#"
            UPDATE wallet_scoring_state
            SET last_scored_ts = ?1
            WHERE id = 1
            "#,
            params![ts],
        )?;

        // If legacy column exists, update it too
        let has_last_ts = {
            let mut has = false;
            let mut stmt = self
                .conn
                .prepare("PRAGMA table_info(wallet_scoring_state)")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(1)?;
                if name == "last_ts" {
                    has = true;
                    break;
                }
            }
            has
        };

        if has_last_ts {
            let _ = self.conn.execute(
                r#"
                UPDATE wallet_scoring_state
                SET last_ts = ?1
                WHERE id = 1
                "#,
                params![ts],
            )?;
        }

        Ok(())
    }

    /// Compute raw deltas from wallet_edges between (since_ts, now_ts].
    pub fn wallet_scoring_compute_deltas(
        &mut self,
        since_ts: i64,
        now_ts: i64,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              src_wallet,
              COUNT(*) AS edges,
              COUNT(DISTINCT mint) AS uniq_mints,
              MIN(ts) AS first_ts,
              MAX(ts) AS last_ts
            FROM wallet_edges
            WHERE ts > ?1 AND ts <= ?2
            GROUP BY src_wallet
            "#,
        )?;

        let mut rows = stmt.query(params![since_ts, now_ts])?;
        let mut out: Vec<(String, i64)> = Vec::new();

        while let Some(r) = rows.next()? {
            let wallet: String = r.get(0)?;
            let edges: i64 = r.get(1)?;
            let uniq_mints: i64 = r.get(2)?;
            let first_ts: i64 = r.get(3)?;
            let last_ts: i64 = r.get(4)?;

            if edges <= 0 {
                continue;
            }

            // ----- base gain -----
            // Small base reward per edges, not crazy.
            let mut delta: i64 = (edges / 6).max(1); // 6 edges -> +1

            // ----- mint diversity reward -----
            if uniq_mints >= 3 {
                delta += 10;
            }
            if uniq_mints >= 10 {
                delta += 20;
            }
            if uniq_mints >= 25 {
                delta += 25;
            }

            // ----- concentration / bot penalty (STRONG) -----
            if uniq_mints <= 1 {
                // Heavy hammer: 100+ edges on one mint in a window is almost always automation.
                if edges >= 200 {
                    delta -= 400;
                } else if edges >= 120 {
                    delta -= 250;
                } else if edges >= 100 {
                    delta -= 175;
                } else if edges >= 60 {
                    delta -= 90;
                } else if edges >= 40 {
                    delta -= 45;
                }

                // Single-mint wallets should never go positive.
                delta = delta.min(0);
            } else {
                // ratio-based soft cap: uniq_mints/edges
                let ratio_times_1000 = (uniq_mints * 1000) / edges.max(1);
                if edges >= 80 && ratio_times_1000 <= 25 {
                    delta = delta.min(5);
                } else if edges >= 50 && ratio_times_1000 <= 50 {
                    delta = delta.min(12);
                }
            }

            // ----- burst cap -----
            let span = (last_ts - first_ts).max(1);
            if edges >= 120 && span <= 90 {
                delta = delta.min(5);
            } else if edges >= 80 && span <= 120 {
                delta = delta.min(10);
            }

            // ----- final per-run cap -----
            // widen negative clamp because we now use heavier penalties
            delta = delta.clamp(-500, 50);

            out.push((wallet, delta));
        }

        Ok(out)
    }

    /// Smooth update:
    /// - ensures wallet exists
    /// - decays old score slightly
    /// - adds delta
    pub fn wallet_score_smooth_update(&mut self, wallet: &str, delta: i64, ts: i64) -> Result<()> {
        self.touch_wallet(wallet, ts)?;

        // score = floor(score*0.98) + delta
        self.conn.execute(
            r#"
            UPDATE wallets
            SET score = CAST(score * 0.98 AS INTEGER) + ?2,
                last_seen_ts = CASE
                  WHEN last_seen_ts IS NULL OR last_seen_ts < ?3 THEN ?3
                  ELSE last_seen_ts
                END
            WHERE wallet = ?1
            "#,
            params![wallet, delta, ts],
        )?;
        Ok(())
    }
    // =========================
    // Schema/Events
    // =========================
    pub fn events_5m(&self, now_ts: i64, mint: &str) -> rusqlite::Result<u64> {
        let since = now_ts.saturating_sub(300);
        self.conn.query_row(
            "SELECT COUNT(*) FROM wallet_edges WHERE mint = ?1 AND ts >= ?2",
            rusqlite::params![mint, since],
            |row| row.get::<_, i64>(0).map(|v| v as u64),
        )
    }

    /// Mint edge stats in last 5 minutes.
    pub fn mint_edge_stats_5m(
        &mut self,
        now_ts: i64,
        mint: &str,
    ) -> anyhow::Result<(u64, u64, f64)> {
        // returns: (uniq_src_wallets, edges_total, edges_per_wallet)
        let q = r#"
        SELECT
          COUNT(DISTINCT src_wallet) AS uniq_src,
          COUNT(*) AS edges_total,
          1.0*COUNT(*)/NULLIF(COUNT(DISTINCT src_wallet),0) AS epw
        FROM wallet_edges
        WHERE mint=?1 AND ts >= (?2 - 300);
    "#;

        let mut stmt = self.conn.prepare(q)?;
        let row = stmt.query_row([mint, &now_ts.to_string()], |r| {
            let uniq: i64 = r.get(0)?;
            let edges: i64 = r.get(1)?;
            let epw: f64 = r.get(2)?;
            Ok((uniq as u64, edges as u64, epw))
        })?;

        Ok(row)
    }

    /// Mint concentration stats in last 5 minutes.
    pub fn mint_concentration_5m(
        &mut self,
        now_ts: i64,
        mint: &str,
    ) -> anyhow::Result<(u64, u64, f64, u64, f64)> {
        // returns: (total_edges, top1_edges, top1_pct, top5_edges, top5_pct)
        let q = r#"
    WITH edges AS (
      SELECT src_wallet, COUNT(*) AS n
      FROM wallet_edges
      WHERE mint=?1 AND ts >= (?2 - 300)
      GROUP BY src_wallet
    ),
    tot AS ( SELECT SUM(n) AS total_edges FROM edges ),
    ranked AS (
      SELECT src_wallet, n,
             ROW_NUMBER() OVER (ORDER BY n DESC) AS rnk
      FROM edges
    )
    SELECT
      COALESCE((SELECT total_edges FROM tot),0) AS total_edges,
      COALESCE((SELECT n FROM ranked WHERE rnk=1),0) AS top1_edges,
      CASE
        WHEN COALESCE((SELECT total_edges FROM tot),0)=0 THEN 0.0
        ELSE 1.0*(SELECT n FROM ranked WHERE rnk=1)/(SELECT total_edges FROM tot)
      END AS top1_pct,
      COALESCE((SELECT SUM(n) FROM ranked WHERE rnk<=5),0) AS top5_edges,
      CASE
        WHEN COALESCE((SELECT total_edges FROM tot),0)=0 THEN 0.0
        ELSE 1.0*(SELECT SUM(n) FROM ranked WHERE rnk<=5)/(SELECT total_edges FROM tot)
      END AS top5_pct;
    "#;

        let mut stmt = self.conn.prepare(q)?;
        let row = stmt.query_row([mint, &now_ts.to_string()], |r| {
            let total: i64 = r.get(0)?;
            let top1: i64 = r.get(1)?;
            let top1_pct: f64 = r.get(2)?;
            let top5: i64 = r.get(3)?;
            let top5_pct: f64 = r.get(4)?;
            Ok((total as u64, top1 as u64, top1_pct, top5 as u64, top5_pct))
        })?;

        Ok(row)
    }

    /// Mint SOL flow in last 5 minutes.
    pub fn mint_sol_flow_5m(&mut self, now_ts: i64, mint: &str) -> anyhow::Result<f64> {
        let q = r#"
      SELECT COALESCE(SUM(COALESCE(sol,0)),0)
      FROM wallet_edges
      WHERE mint=?1 AND ts >= (?2 - 300);
    "#;
        let mut stmt = self.conn.prepare(q)?;
        let sol: f64 = stmt.query_row([mint, &now_ts.to_string()], |r| r.get(0))?;
        Ok(sol)
    }

    // =========================
    // Snapshots + calls
    // =========================

    pub fn insert_snapshot(
        &mut self,
        ts: i64,
        mint: &str,
        fdv_usd: Option<f64>,
        tx_5m: Option<u64>,
        score: i32,
        signers_5m: u64,
        events: usize,
        first_seen: u64,
        is_active: bool,
        is_call: bool,
    ) -> Result<()> {
        if std::env::var("DISABLE_SNAPSHOTS").ok().as_deref() == Some("1") {
            return Ok(());
        }
        self.conn.execute(
            r#"
    INSERT INTO mint_snapshots
      (ts, mint, fdv_usd, tx_5m, score, signers_5m, events, first_seen, is_active, is_call)
    VALUES
      (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
    ON CONFLICT(mint, ts) DO UPDATE SET
      fdv_usd     = excluded.fdv_usd,
      tx_5m       = excluded.tx_5m,
      score       = excluded.score,
      signers_5m  = excluded.signers_5m,
      events      = excluded.events,
      first_seen  = excluded.first_seen,
      is_active   = excluded.is_active,
      is_call     = excluded.is_call
    "#,
            params![
                ts,
                mint,
                fdv_usd,
                tx_5m.map(|x| x as i64),
                score,
                signers_5m as i64,
                events as i64,
                first_seen as i64,
                if is_active { 1 } else { 0 },
                if is_call { 1 } else { 0 },
            ],
        )?;

        Ok(())
    }

    pub fn insert_call_debug(
        &mut self,
        call_ts: i64,
        mint: &str,
        fdv_usd: f64,
        score: i32,
        tx5: u64,
        signers: u64,
        events: usize,
        ss: u64, // signer_strength
        age_sec: u64,
        wallet_delta: i32,
        conc_total_edges: i64,
        conc_top1_edges: i64,
        conc_top5_edges: i64,
        conc_top1_pct: f64,
        conc_top5_pct: f64,
        top1_wallet: &str,
        top1_wallet_score: i64,
        top1_is_whale: bool,
        ge10_cnt: i64,
        veto_bad_top1: bool,
        max_gap_secs: i64, // ✅ NEW
        revive: bool,
        gambol: bool,
        conc: bool,
    ) -> anyhow::Result<()> {
        use rusqlite::params;

        self.conn.execute(
            r#"
        INSERT INTO call_debug(
          call_ts, mint,
          fdv_usd, score, tx_5m, signers_5m, events,
          signer_strength, age_sec, wallet_delta,
          conc_total_edges, conc_top1_edges, conc_top5_edges,
          conc_top1_pct, conc_top5_pct,
          top1_wallet, top1_wallet_score, top1_is_whale,
          ge10_cnt, veto_bad_top1,
          max_gap_secs,            -- ✅ NEW
          revive, gambol, conc
        )
        VALUES(
          ?1, ?2,
          ?3, ?4, ?5, ?6, ?7,
          ?8, ?9, ?10,
          ?11, ?12, ?13,
          ?14, ?15,
          ?16, ?17, ?18,
          ?19, ?20,
          ?21,                 -- ✅ NEW max_gap_secs
          ?22, ?23, ?24
        )
        ON CONFLICT(call_ts, mint) DO UPDATE SET
          fdv_usd=excluded.fdv_usd,
          score=excluded.score,
          tx_5m=excluded.tx_5m,
          signers_5m=excluded.signers_5m,
          events=excluded.events,
          signer_strength=excluded.signer_strength,
          age_sec=excluded.age_sec,
          wallet_delta=excluded.wallet_delta,
          conc_total_edges=excluded.conc_total_edges,
          conc_top1_edges=excluded.conc_top1_edges,
          conc_top5_edges=excluded.conc_top5_edges,
          conc_top1_pct=excluded.conc_top1_pct,
          conc_top5_pct=excluded.conc_top5_pct,
          top1_wallet=excluded.top1_wallet,
          top1_wallet_score=excluded.top1_wallet_score,
          top1_is_whale=excluded.top1_is_whale,
          ge10_cnt=excluded.ge10_cnt,
          veto_bad_top1=excluded.veto_bad_top1,
          max_gap_secs=excluded.max_gap_secs,  -- ✅ NEW
          revive=excluded.revive,
          gambol=excluded.gambol,
          conc=excluded.conc
        "#,
            params![
                call_ts,
                mint,
                fdv_usd,
                score,
                tx5 as i64,
                signers as i64,
                events as i64,
                ss as i64,
                age_sec as i64,
                wallet_delta as i64,
                conc_total_edges,
                conc_top1_edges,
                conc_top5_edges,
                conc_top1_pct,
                conc_top5_pct,
                top1_wallet,
                top1_wallet_score,
                if top1_is_whale { 1 } else { 0 },
                ge10_cnt,
                if veto_bad_top1 { 1 } else { 0 },
                max_gap_secs, // ✅ NEW (goes with ?21)
                if revive { 1 } else { 0 },
                if gambol { 1 } else { 0 },
                if conc { 1 } else { 0 },
            ],
        )?;

        Ok(())
    }

    pub fn insert_call_debug_top_wallet(
        &mut self,
        call_ts: i64,
        mint: &str,
        rank: i64,
        wallet: &str,
        edges: i64,
        wallet_score: i64,
        is_whale: bool,
    ) -> anyhow::Result<()> {
        use rusqlite::params;

        self.conn.execute(
            r#"
        INSERT INTO call_debug_top_wallets(
          call_ts, mint, rank, wallet, edges, wallet_score, is_whale
        )
        VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(call_ts, mint, rank) DO UPDATE SET
          wallet=excluded.wallet,
          edges=excluded.edges,
          wallet_score=excluded.wallet_score,
          is_whale=excluded.is_whale
        "#,
            params![
                call_ts,
                mint,
                rank,
                wallet,
                edges,
                wallet_score,
                if is_whale { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    pub fn insert_call(
        &mut self,
        ts: i64,
        mint: &str,
        fdv_usd: f64,
        score: i32,
        tx_5m: u64,
        signers_5m: u64,
        events: usize,
        tag: Option<&str>,
    ) -> Result<()> {
        eprintln!(
        "DBG insert_call: ts={} mint={} fdv_usd={} score={} tx_5m={} signers_5m={} events={} tag={}",
        ts,
        mint,
        fdv_usd,
        score,
        tx_5m,
        signers_5m,
        events,
        tag.unwrap_or("")
    );

        let n = self.conn.execute(
            r#"
        INSERT OR IGNORE INTO calls (ts, mint, fdv_usd, score, tx_5m, signers_5m, events, tag)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
            params![
                ts,
                mint,
                fdv_usd,
                score,
                tx_5m as i64,
                signers_5m as i64,
                events as i64,
                tag.unwrap_or("")
            ],
        )?;

        eprintln!("DBG insert_call: rows_affected={}", n);
        Ok(())
    }

    // =========================
    // Dedupe
    // =========================

    pub fn seen_sig(&mut self, sig: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT 1 FROM seen_sigs WHERE sig=?1 LIMIT 1")?;
        let mut rows = stmt.query(params![sig])?;
        Ok(rows.next()?.is_some())
    }

    pub fn last_call_ts_for_mint(&self, mint: &str) -> anyhow::Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT MAX(ts) FROM calls WHERE mint = ?1")?;
        let ts: Option<i64> = stmt.query_row([mint], |row| row.get(0)).optional()?;
        Ok(ts)
    }

    pub fn mark_sig(&mut self, ts: i64, sig: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO seen_sigs (sig, ts) VALUES (?1, ?2)",
            params![sig, ts],
        )?;
        Ok(())
    }

    pub fn signers_5m(&self, now_ts: i64, mint: &str) -> Result<u64> {
        let since = now_ts - 300;
        let mut stmt = self.conn.prepare(
            r#"
        SELECT COUNT(DISTINCT src_wallet)
        FROM wallet_edges
        WHERE mint = ?1
          AND ts >= ?2
          AND ts <= ?3
        "#,
        )?;
        let n: i64 = stmt.query_row(params![mint, since, now_ts], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn uniq_sigs_5m(&self, now_ts: i64, mint: &str) -> anyhow::Result<u64> {
        let since = now_ts - 300;
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(DISTINCT sig)
         FROM wallet_edges
         WHERE mint = ?1
           AND ts >= ?2
           AND sig IS NOT NULL
           AND sig != ''",
        )?;
        let n: u64 = stmt.query_row((mint, since), |r| r.get(0))?;
        Ok(n)
    }
}
