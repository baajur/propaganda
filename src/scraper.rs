use crate::db::{ ProvideArticles, Article };
use anyhow::anyhow;
use std::time::Duration;
use xactor::*;

#[message(result = "()")]
#[derive(Clone, Debug)]
struct DumpArticleUrls;

#[message(result = "()")]
#[derive(Clone, Debug)]
struct FetchTopArticle;

pub struct Scraper {
    pool: sqlx::SqlitePool,
}

impl Scraper {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    async fn dump_article_urls(&self) -> Result<()> {
        let urls = self
            .pool
            .acquire()
            .await?
            .get_articles(0, 100)
            .await?
            .into_iter()
            .map(|a| a.url)
            .collect::<Vec<String>>()
            .join("\n");
        println!("dump_article_urls {}", urls);
        Ok(())
    }

    async fn fetch_top_article(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        if let Some(outdated) = conn.get_outdated_articles(1).await?.get(0) {
            // TODO saving time as i32 is not good
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let timestamp = timestamp as i32;

            conn.update_article(&outdated.url, timestamp).await?;

            let html = surf_get_string(&outdated.url).await?;

            let snapshot_is_outdated = conn
                .get_youngest_snaphot(&outdated)
                .await?
                .filter(|s| &s.html == &html)
                .is_none();

            if snapshot_is_outdated {
                conn.insert_snapshot(&outdated, timestamp, &html).await?;
            }
        }
        Ok(())
    }

    async fn fetch_whatthecommit(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        let url = "http://whatthecommit.com/";
        let article = conn.insert_article(url).await?;
        self.insert_snapshot(&mut conn, &article).await?;
        Ok(())
    }

    async fn insert_snapshot(&self, provider: &mut sqlx::pool::PoolConnection<sqlx::SqliteConnection>, article: &Article) -> Result<()> {
        let html = surf_get_string(&article.url).await?;
        provider.insert_snapshot(article, self.timestamp(), &html).await?;
        Ok(())
    }

    fn timestamp(&self) -> i32 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i32
    }
}

#[async_trait::async_trait]
impl Actor for Scraper {
    async fn started(&mut self, ctx: &mut Context<Self>) -> Result<()> {
        ctx.send_interval(FetchTopArticle, Duration::from_secs(60));
        self.fetch_whatthecommit().await?;
        self.fetch_whatthecommit().await?;
        self.fetch_whatthecommit().await?;
        self.fetch_whatthecommit().await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Handler<DumpArticleUrls> for Scraper {
    async fn handle(&mut self, _ctx: &mut Context<Self>, _msg: DumpArticleUrls) -> () {
        if let Err(err) = self.dump_article_urls().await {
            tide::log::error!("{}", err);
        }
    }
}

#[async_trait::async_trait]
impl Handler<FetchTopArticle> for Scraper {
    async fn handle(&mut self, _ctx: &mut Context<Self>, _msg: FetchTopArticle) -> () {
        if let Err(err) = self.fetch_top_article().await {
            tide::log::error!("{}", err);
        }
    }
}

async fn surf_get_string(uri: impl AsRef<str>) -> Result<String> {
    surf::url::Url::parse(uri.as_ref())?;
    surf::get(uri)
        .recv_string()
        .await
        .map_err(|err| anyhow!(err))
}

fn compare_article_fulltext(a: &str, b: &str) -> bool {
    get_article_fulltext(a) == get_article_fulltext(b)
}

/// for tagesschau.de
fn get_article_fulltext(html: &str) -> String {
    let fragment = scraper::Html::parse_fragment(&html);
    let mut fulltext = String::new();
    
    if let Ok(selector) = scraper::Selector::parse("div.storywrapper") {
        for element in fragment.select(&selector) {
            for text in element.text() {
                let text = text.trim();
                if !text.is_empty() {
                    fulltext.push_str(text);
                    fulltext.push_str("\n");
                }
            }
        }
    } else if let Ok(selector) = scraper::Selector::parse("div#content") {
        for element in fragment.select(&selector) {
            for text in element.text() {
                let text = text.trim();
                if !text.is_empty() {
                    fulltext.push_str(text);
                    fulltext.push_str("\n");
                }
            }
        }
    }
    fulltext
}
