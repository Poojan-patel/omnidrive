// GET / — the home page. In M2 this is a static hello-world render; M3 grows a
// real data dependency (the connected-accounts list from SQLite).

use askama::Template;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

pub async fn index() -> IndexTemplate {
    IndexTemplate
}
