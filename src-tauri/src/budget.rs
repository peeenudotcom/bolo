//! Local estimated spending guard. Reservations are durable before network I/O.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::{Mutex, OnceLock}};
static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());
#[derive(Clone, Serialize, Deserialize)]
struct Ledger { limit_cents: u64, months: BTreeMap<String, u64> }
impl Default for Ledger {
    fn default() -> Self { Self { limit_cents: 2000, months: BTreeMap::new() } }
}
#[derive(Serialize)]
pub struct View { pub month: String, pub limit_cents: u64, pub used_micros: u64 }
pub struct Reservation { month: String, amount: u64 }
pub fn init(path: PathBuf) { let _ = PATH.set(path); }
fn month() -> String { chrono::Utc::now().format("%Y-%m").to_string() }
fn read(path: &Path) -> Result<Ledger> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Budget record is damaged. Cloud requests are blocked; keep the file for repair."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(e).context("Cannot read budget record. Cloud requests are blocked."),
    }
}
fn write(path: &Path, ledger: &Ledger) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&serde_json::to_vec(ledger)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path).context("Cannot save budget record. Cloud requests are blocked.")
}
fn update<T>(f: impl FnOnce(&mut Ledger) -> Result<T>) -> Result<T> {
    let _guard = LOCK.lock().map_err(|_| anyhow::anyhow!("Budget tracker unavailable. Restart Bolo."))?;
    let path = PATH.get().context("Budget tracker is not ready")?;
    let mut ledger = read(path)?;
    let value = f(&mut ledger)?;
    write(path, &ledger)?;
    Ok(value)
}
pub fn view() -> Result<View> {
    update(|l| { let m = month(); Ok(View { used_micros: *l.months.get(&m).unwrap_or(&0), month:m, limit_cents:l.limit_cents }) })
}
pub fn set_limit(cents: u64) -> Result<View> {
    ensure!(cents <= 1_000_000, "Choose a monthly limit between $0 and $10,000.");
    update(|l| { l.limit_cents=cents; Ok(()) })?;
    view()
}
fn charge(l: &mut Ledger, m: &str, amount: u64) -> Result<()> {
    let spent = l.months.entry(m.into()).or_default();
    let total = spent.checked_add(amount).context("Budget total is too large")?;
    ensure!(total <= l.limit_cents.saturating_mul(10_000),
        "Monthly Bolo budget reached, or too little remains for this request. Change the limit in Settings → Monthly budget, or use offline recognition with AI cleanup off. Your recording is safe in History.");
    *spent = total;
    Ok(())
}
pub fn reserve(amount: u64) -> Result<Option<Reservation>> {
    // Unit/live test harnesses have no application budget file.
    #[cfg(test)]
    if PATH.get().is_none() { return Ok(None); }
    let m=month();
    update(|l| charge(l, &m, amount))?;
    Ok(Some(Reservation { month:m, amount }))
}
pub fn settle(reservation: Option<Reservation>, actual: u64) -> Result<()> {
    let Some(r)=reservation else { return Ok(()); };
    update(|l| {
        reconcile(l, r, actual);
        Ok(())
    })
}
fn reconcile(l: &mut Ledger, r: Reservation, actual: u64) {
    let spent=l.months.entry(r.month).or_default();
    *spent=spent.saturating_sub(r.amount).saturating_add(actual);
}
// Prices checked 2026-11-07. USD millionths; intentionally ignore cache discounts.
// https://developers.openai.com/api/docs/pricing
pub fn speech_cost(samples: usize) -> u64 {
    ((samples as u64).saturating_mul(4500)).div_ceil(16_000 * 60).max(1)
}
pub fn text_cost(model: &str, input: u64, output: u64) -> Result<u64> {
    let (i,o)=match model {
        "gpt-6-sol" | "gpt-6.1-sol" => (2,10),
        _ => anyhow::bail!("Budget pricing is unavailable for this cleanup model. Select gpt-6-sol in advanced settings."),
    };
    Ok(input.saturating_mul(i).saturating_add(output.saturating_mul(o)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn budget_blocks_before_overspend_and_zero_blocks_cloud() {
        let mut l=Ledger { limit_cents:1, ..Ledger::default() };
        charge(&mut l,"2026-11",6000).unwrap();
        assert!(charge(&mut l,"2026-11",4001).is_err());
        charge(&mut l,"2026-11",4000).unwrap();
        assert!(charge(&mut l,"2026-11",1).is_err());
        charge(&mut l,"2026-12",1).unwrap();
        assert!(charge(&mut l,"2026-11",1).is_err()); // clock rollback cannot reset spending
        l.limit_cents=0;
        assert!(charge(&mut l,"2027-01",1).is_err());
    }
    #[test] fn settlement_stays_in_original_month_and_preserves_other_reservations() {
        let mut l=Ledger::default();
        charge(&mut l,"2026-11",50_000).unwrap();
        charge(&mut l,"2026-11",40_000).unwrap();
        charge(&mut l,"2026-12",10_000).unwrap();
        reconcile(&mut l,Reservation {month:"2026-11".into(), amount:50_000},3000);
        assert_eq!(l.months["2026-11"],43_000);
        assert_eq!(l.months["2026-12"],10_000);
        // The unfinished request's 40,000 stays charged after restart.
        let restored: Ledger=serde_json::from_slice(&serde_json::to_vec(&l).unwrap()).unwrap();
        assert_eq!(restored.months["2026-11"],43_000);
    }
    #[test] fn concurrent_reservations_cannot_exceed_limit() {
        let l=std::sync::Arc::new(Mutex::new(Ledger {limit_cents:1,..Ledger::default()}));
        let threads:Vec<_>=(0..20).map(|_| { let l=l.clone(); std::thread::spawn(move || charge(&mut l.lock().unwrap(),"2026-11",1000).is_ok()) }).collect();
        assert_eq!(threads.into_iter().filter(|t| t.thread().id()!=std::thread::current().id()).map(|t| usize::from(t.join().unwrap())).sum::<usize>(),10);
        assert_eq!(l.lock().unwrap().months["2026-11"],10_000);
    }
    #[test] fn costs_and_unknown_model() {
        assert_eq!(speech_cost(16_000*60),4500);
        assert_eq!(text_cost("gpt-6-sol",1000,100).unwrap(),3000);
        assert!(text_cost("unknown",1,1).is_err());
    }
    #[test] fn ledger_survives_restart_and_rejects_corruption() {
        let dir=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.scratch/budget-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path=dir.join("usage.json");
        let mut l=Ledger::default(); charge(&mut l,"2026-11",5678).unwrap();
        write(&path,&l).unwrap();
        assert_eq!(read(&path).unwrap().months["2026-11"],5678);
        std::fs::write(&path,b"broken").unwrap();
        assert!(read(&path).is_err());
    }
}
