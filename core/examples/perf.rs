//! Synthetic workload only; no real vault or passwords.
use secback_core::{format::{Project,Account},merge::{merge,MergeOptions},vault::Store};
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},time::Instant};
struct Meter;
static LIVE:AtomicUsize=AtomicUsize::new(0);static PEAK:AtomicUsize=AtomicUsize::new(0);
unsafe impl GlobalAlloc for Meter {
 unsafe fn alloc(&self,l:Layout)->*mut u8{let p=System.alloc(l);if !p.is_null(){let n=LIVE.fetch_add(l.size(),Ordering::Relaxed)+l.size();PEAK.fetch_max(n,Ordering::Relaxed);}p}
 unsafe fn dealloc(&self,p:*mut u8,l:Layout){LIVE.fetch_sub(l.size(),Ordering::Relaxed);System.dealloc(p,l)}
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{let r=System.realloc(p,l,n);if !r.is_null(){if n>=l.size(){let v=LIVE.fetch_add(n-l.size(),Ordering::Relaxed)+n-l.size();PEAK.fetch_max(v,Ordering::Relaxed);}else{LIVE.fetch_sub(l.size()-n,Ordering::Relaxed);}}r}
}
#[global_allocator]static ALLOC:Meter=Meter;
fn measure<T>(n:usize,op:&str,f:impl FnOnce()->T)->T{PEAK.store(LIVE.load(Ordering::Relaxed),Ordering::Relaxed);let start=Instant::now();let value=f();println!("{{\"n\":{n},\"op\":\"{op}\",\"ms\":{:.3},\"peakHeapBytes\":{}}}",start.elapsed().as_secs_f64()*1000.,PEAK.load(Ordering::Relaxed));value}
fn main(){for n in [100,1000,10000]{
 let dir=std::env::temp_dir().join(format!("secback-perf-{}-{n}",std::process::id()));std::fs::create_dir(&dir).unwrap();let path=dir.join("synthetic.properties");
 let mut store=Store::new();store.load_or_create(&path,"synthetic-perf-password",true).unwrap();
 for i in 0..n{let mut p=Project::new_with_id((i as u128).to_le_bytes());p.name=format!("虚构项目{i}");p.slots[0]=Account{user:format!("user{i}"),pass:"synthetic-secret".into(),remark:"benchmark".into()};store.projects_mut().push(p);}
 measure(n,"save",||store.save(&path).unwrap());store.clear_secure();measure(n,"unlock",||store.load(&path,"synthetic-perf-password").unwrap());
 measure(n,"select",||assert!(store.find_index(&((n-1) as u128).to_le_bytes()).is_some()));
 measure(n,"search",||{std::hint::black_box(store.projects().iter().filter(|p|p.name.contains("99")||p.slots.iter().any(|a|a.user.contains("99"))).count());});
 let incoming=store.projects().to_vec();let plan=measure(n,"import",||merge(store.projects(),&incoming,&MergeOptions::default()));assert_eq!(plan.added,0);
 drop(plan);drop(store);drop(incoming);std::fs::remove_dir_all(dir).unwrap();
}}
