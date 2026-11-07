//! F11 AC-3: Zwischenablage auf KDE über ext-data-control-v1. Ein eigener Thread mit eigener
//! Wayland-Verbindung bedient die Abholung, während die Hauptschleife Umschalt+Einfg drückt.
//! Vor dem Setzen wird der alte Inhalt (alle Formate) gelesen und danach wiederhergestellt.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{self as device, ExtDataControlDeviceV1};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{self as offer, ExtDataControlOfferV1};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{self as source, ExtDataControlSourceV1};

use crate::uinput::Zwischenablage;

const TEXT: [&str; 4] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"];
/// Klipper (KDE) nimmt so markierte Einträge nicht in den Verlauf auf
const GEHEIM: &str = "x-kde-passwordManagerHint";
/// ponytail: Obergrenze pro gesichertem Format, damit ein riesiges Bild die Ausgabe nicht aufhält
const MAX_FORMAT: usize = 16 * 1024 * 1024;

type Inhalt = Vec<(String, Vec<u8>)>;

#[derive(Default)]
struct Zustand {
    mimes: HashMap<ObjectId, Vec<String>>,
    aktuell: Option<ExtDataControlOfferV1>,
    /// Auswahl (primary selection): viele Terminals fügen bei Umschalt+Einfg diese ein
    auswahl: Option<ExtDataControlOfferV1>,
    /// Was unsere eigene Quelle ausliefert
    liefern: HashMap<ObjectId, Inhalt>,
    /// Wie oft ein Programm unseren Inhalt abgeholt hat
    abgeholt: u64,
}

/// Höchstens so lange auf die Abholung warten, bevor die alte Zwischenablage zurückkommt.
/// Thunderbird und Firefox brauchen deutlich länger als Terminals.
pub const ABHOLUNG_MAX: Duration = Duration::from_millis(1000);

enum Befehl {
    /// Text, optional HTML (F10)
    Setzen(String, Option<String>, Sender<Result<(), String>>),
    Zurueck(Sender<()>),
}

/// Handle für die Hauptschleife; die Arbeit macht der Thread
pub struct KdeAblage {
    tx: Sender<Befehl>,
}

impl KdeAblage {
    pub fn starten() -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("Keine Wayland-Sitzung: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<Zustand>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).map_err(|e| format!("Kein wl_seat: {e}"))?;
        let mgr: ExtDataControlManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("Compositor kann keine Zwischenablage (ext_data_control_v1): {e}"))?;
        let geraet = mgr.get_data_device(&seat, &qh, ());
        let mut z = Zustand::default();
        queue.roundtrip(&mut z).map_err(|e| e.to_string())?;
        let (tx, rx) = channel();
        std::thread::spawn(move || schleife(conn, queue, z, mgr, geraet, rx));
        Ok(KdeAblage { tx })
    }
}

impl Zwischenablage for KdeAblage {
    fn setzen(&mut self, text: &str) -> Result<(), String> {
        let (a, b) = channel();
        self.tx.send(Befehl::Setzen(text.to_string(), None, a)).map_err(|_| "Zwischenablage beendet")?;
        b.recv_timeout(Duration::from_secs(2)).map_err(|_| "Zwischenablage antwortet nicht".to_string())?
    }
    fn setzen_html(&mut self, html: &str, text: &str) -> Result<(), String> {
        let (a, b) = channel();
        self.tx.send(Befehl::Setzen(text.to_string(), Some(html.to_string()), a)).map_err(|_| "Zwischenablage beendet")?;
        b.recv_timeout(Duration::from_secs(2)).map_err(|_| "Zwischenablage antwortet nicht".to_string())?
    }
    fn zuruecksetzen(&mut self) {
        let (a, b) = channel();
        if self.tx.send(Befehl::Zurueck(a)).is_ok() {
            let _ = b.recv_timeout(Duration::from_secs(2));
        }
    }
}

fn schleife(conn: Connection, mut queue: EventQueue<Zustand>, mut z: Zustand, mgr: ExtDataControlManagerV1, geraet: ExtDataControlDeviceV1, rx: Receiver<Befehl>) {
    let qh = queue.handle();
    let mut gesichert: Inhalt = vec![];
    let mut gesichert_auswahl: Inhalt = vec![];
    let mut abgeholt_vorher = 0;
    loop {
        pumpen(&conn, &mut queue, &mut z, 20);
        match rx.try_recv() {
            Ok(Befehl::Setzen(text, html, antwort)) => {
                abgeholt_vorher = z.abgeholt;
                gesichert = z.aktuell.clone().map(|o| lesen(&conn, &mut queue, &mut z, &o)).unwrap_or_default();
                gesichert_auswahl = z.auswahl.clone().map(|o| lesen(&conn, &mut queue, &mut z, &o)).unwrap_or_default();
                let mut inhalt: Inhalt = TEXT.iter().map(|m| (m.to_string(), text.as_bytes().to_vec())).collect();
                if let Some(h) = html {
                    // F10: HTML zuerst anbieten, Programme nehmen das erste Format, das sie kennen
                    inhalt.insert(0, ("text/html".into(), h.into_bytes()));
                }
                inhalt.push((GEHEIM.into(), b"secret".to_vec()));
                anbieten(&mgr, &geraet, &qh, &mut z, inhalt.clone(), false);
                anbieten(&mgr, &geraet, &qh, &mut z, inhalt, true);
                let _ = conn.flush();
                let _ = antwort.send(Ok(()));
            }
            Ok(Befehl::Zurueck(antwort)) => {
                // Erst zurückstellen, wenn das Programm abgeholt hat, sonst bekommt es den alten Inhalt oder nichts
                let ende = std::time::Instant::now() + ABHOLUNG_MAX;
                while z.abgeholt == abgeholt_vorher && std::time::Instant::now() < ende {
                    pumpen(&conn, &mut queue, &mut z, 10);
                }
                if z.abgeholt == abgeholt_vorher {
                    // Fehlersuche: das Programm hat das Einfügen-Tastenkürzel nicht als Einfügen verstanden
                    eprintln!("tippkraft: Zwischenablage wurde nicht abgeholt (nach {} ms zurückgestellt)", ABHOLUNG_MAX.as_millis());
                }
                // weitere Formate desselben Einfügens (erst text/html, dann text/plain)
                pumpen(&conn, &mut queue, &mut z, 50);
                for (inhalt, auswahl) in [(std::mem::take(&mut gesichert), false), (std::mem::take(&mut gesichert_auswahl), true)] {
                    if !inhalt.is_empty() {
                        anbieten(&mgr, &geraet, &qh, &mut z, inhalt, auswahl);
                    } else if auswahl {
                        geraet.set_primary_selection(None);
                    } else {
                        geraet.set_selection(None);
                    }
                }
                let _ = conn.flush();
                let _ = antwort.send(());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            Err(_) => {}
        }
    }
}

fn anbieten(mgr: &ExtDataControlManagerV1, geraet: &ExtDataControlDeviceV1, qh: &QueueHandle<Zustand>, z: &mut Zustand, inhalt: Inhalt, auswahl: bool) {
    let q = mgr.create_data_source(qh, ());
    for (m, _) in &inhalt {
        q.offer(m.clone());
    }
    z.liefern.insert(q.id(), inhalt);
    if auswahl {
        geraet.set_primary_selection(Some(&q));
    } else {
        geraet.set_selection(Some(&q));
    }
}

/// Alle Formate des aktuellen Inhalts lesen. Die Quelle ist eine andere App, also während des Lesens weiter pumpen.
fn lesen(conn: &Connection, queue: &mut EventQueue<Zustand>, z: &mut Zustand, o: &ExtDataControlOfferV1) -> Inhalt {
    let mut out = vec![];
    for m in z.mimes.get(&o.id()).cloned().unwrap_or_default() {
        let Ok((r, w)) = rustix::pipe::pipe() else { continue };
        o.receive(m.clone(), w.as_fd());
        drop(w);
        let _ = conn.flush();
        let mut datei = std::fs::File::from(r);
        let _ = rustix::fs::fcntl_setfl(&datei, rustix::fs::OFlags::NONBLOCK);
        let mut daten = vec![];
        let mut puffer = [0u8; 65536];
        let ende = std::time::Instant::now() + Duration::from_millis(500);
        loop {
            match datei.read(&mut puffer) {
                Ok(0) => break,
                Ok(n) if daten.len() + n <= MAX_FORMAT => daten.extend_from_slice(&puffer[..n]),
                Ok(_) => {
                    daten.clear();
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < ende => pumpen(conn, queue, z, 5),
                Err(_) => break,
            }
        }
        if !daten.is_empty() {
            out.push((m, daten));
        }
    }
    out
}

fn pumpen(conn: &Connection, queue: &mut EventQueue<Zustand>, z: &mut Zustand, ms: i64) {
    let _ = conn.flush();
    if let Some(guard) = queue.prepare_read() {
        let fd = guard.connection_fd();
        let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
        let t = rustix::time::Timespec { tv_sec: 0, tv_nsec: ms * 1_000_000 };
        if rustix::event::poll(&mut fds, Some(&t)).unwrap_or(0) > 0 {
            let _ = guard.read();
        }
    }
    let _ = queue.dispatch_pending(z);
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Zustand {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_seat::WlSeat, ()> for Zustand {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ExtDataControlManagerV1, ()> for Zustand {
    fn event(_: &mut Self, _: &ExtDataControlManagerV1, _: <ExtDataControlManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ExtDataControlDeviceV1, ()> for Zustand {
    fn event(z: &mut Self, _: &ExtDataControlDeviceV1, e: device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            device::Event::DataOffer { id } => {
                z.mimes.insert(id.id(), vec![]);
            }
            device::Event::Selection { id } => {
                if let Some(alt) = z.aktuell.take() {
                    z.mimes.remove(&alt.id());
                    alt.destroy();
                }
                z.aktuell = id;
            }
            device::Event::PrimarySelection { id } => {
                if let Some(alt) = z.auswahl.take() {
                    z.mimes.remove(&alt.id());
                    alt.destroy();
                }
                z.auswahl = id;
            }
            _ => {}
        }
    }
    event_created_child!(Zustand, ExtDataControlDeviceV1, [device::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ())]);
}
impl Dispatch<ExtDataControlOfferV1, ()> for Zustand {
    fn event(z: &mut Self, o: &ExtDataControlOfferV1, e: offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let offer::Event::Offer { mime_type } = e {
            z.mimes.entry(o.id()).or_default().push(mime_type);
        }
    }
}
impl Dispatch<ExtDataControlSourceV1, ()> for Zustand {
    fn event(z: &mut Self, q: &ExtDataControlSourceV1, e: source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            source::Event::Send { mime_type, fd } => {
                if let Some((_, daten)) = z.liefern.get(&q.id()).and_then(|i| i.iter().find(|(m, _)| *m == mime_type)) {
                    let mut f = std::fs::File::from(OwnedFd::from(fd));
                    let _ = f.write_all(daten);
                    z.abgeholt += 1;
                }
            }
            source::Event::Cancelled => {
                z.liefern.remove(&q.id());
                q.destroy();
            }
            _ => {}
        }
    }
}
