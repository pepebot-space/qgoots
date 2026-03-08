use clap::{Parser, Subcommand};
use qgoots_core::init_db;

#[derive(Parser)]
#[command(name = "qgoots")]
#[command(about = "Local semantic memory engine powered by DuckDB", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true)]
    db: Option<String>,

    #[arg(long, global = true)]
    config: Option<String>,

    #[arg(long, global = true)]
    format: Option<String>,

    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize database and install extensions
    Init,

    /// Manage document collections
    Collection {
        #[command(subcommand)]
        action: CollectionCommands,
    },

    /// Index/re-index documents
    Index {
        collection: Option<String>,
        #[arg(long)]
        force: bool,
    },

    /// Generate embeddings for indexed documents
    Embed {
        collection: Option<String>,
        #[arg(long)]
        force: bool,
    },

    /// BM25 full-text search
    Search {
        query: String,
        #[arg(short = 'n', default_value_t = 10)]
        count: usize,
        #[arg(short = 'c')]
        collection: Option<String>,
        #[arg(long)]
        min_score: Option<f64>,
    },

    /// Vector semantic search
    Vsearch {
        query: String,
        #[arg(short = 'n', default_value_t = 10)]
        count: usize,
        #[arg(short = 'c')]
        collection: Option<String>,
    },

    /// Hybrid search (BM25 + vector + RRF)
    Query {
        query: String,
        #[arg(short = 'n', default_value_t = 10)]
        count: usize,
        #[arg(short = 'c')]
        collection: Option<String>,
        #[arg(long)]
        min_score: Option<f64>,
    },
}

#[derive(Subcommand)]
enum CollectionCommands {
    /// Register a directory as collection
    Add {
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "**/*.md")]
        pattern: String,
    },
    /// List all collections with stats
    List,
    /// Remove a collection
    Remove { name: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup basic tracing
    if cli.verbose {
        tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
    } else {
        tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    }

    let db_path = cli.db.unwrap_or_else(|| "qgoots.duckdb".to_string());
    let vector_dim = 768; // Default, would come from config in full implementation

    match &cli.command {
        Commands::Init => {
            println!("Initializing database at {}", db_path);
            let _conn = init_db(&db_path, vector_dim)?;
            println!("Initialization complete.");
        }
        Commands::Collection { action } => {
            match action {
                CollectionCommands::Add { path, name, pattern } => {
                    let c_name = name.clone().unwrap_or_else(|| {
                        std::path::Path::new(path)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .to_string()
                    });
                    
                    let conn = duckdb::Connection::open(&db_path)?;
                    conn.execute(
                        "INSERT INTO collections (name, path, pattern) VALUES (?, ?, ?)",
                        duckdb::params![c_name, path, pattern],
                    )?;
                    println!("Added collection '{}' at '{}'", c_name, path);
                }
                CollectionCommands::List => {
                    println!("Listing collections...");
                    let conn = duckdb::Connection::open(&db_path)?;
                    let mut stmt = conn.prepare("SELECT name, path, pattern FROM collections")?;
                    let mut rows = stmt.query([])?;
                    while let Some(row) = rows.next()? {
                        let name: String = row.get(0)?;
                        let path: String = row.get(1)?;
                        let pattern: String = row.get(2)?;
                        println!("- {} ({} [{}])", name, path, pattern);
                    }
                }
                CollectionCommands::Remove { name } => {
                    println!("Removing collection: {}", name);
                    let conn = duckdb::Connection::open(&db_path)?;
                    conn.execute("DELETE FROM collections WHERE name = ?", duckdb::params![name])?;
                    println!("Removed collection '{}'", name);
                }
            }
        }
        Commands::Index { collection, force } => {
            println!("Indexing documents... (collection: {:?}, force: {})", collection, force);
            let conn = duckdb::Connection::open(&db_path)?;
            
            let mut filter = String::new();
            if let Some(c) = collection {
                filter = format!(" WHERE name = '{}'", c);
            }
            
            let query = format!("SELECT name, path, pattern FROM collections{}", filter);
            let mut stmt = conn.prepare(&query)?;
            
            struct Col { name: String, path: String, pattern: String }
            let mut cols = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                cols.push(Col {
                    name: row.get(0)?,
                    path: row.get(1)?,
                    pattern: row.get(2)?,
                });
            }
            
            for col in cols {
                println!("Indexing collection '{}'", col.name);
                let mut search_pattern = std::path::PathBuf::from(&col.path);
                search_pattern.push(&col.pattern);
                
                for entry in glob::glob(search_pattern.to_str().unwrap()).expect("Failed to read glob pattern") {
                    match entry {
                        Ok(path) => {
                            if path.is_file() {
                                let content = std::fs::read_to_string(&path)?;
                                let rel_path = path.strip_prefix(&col.path).unwrap_or(&path).to_string_lossy().to_string();
                                println!("  Ingesting {:?}", rel_path);
                                qgoots_core::ingest_document(&conn, &col.name, &rel_path, &content, 512, 50)?;
                            }
                        }
                        Err(e) => println!("{:?}", e),
                    }
                }
            }
            
            println!("Rebuilding FTS index...");
            qgoots_core::db::recreate_fts_index(&conn)?;
            println!("Indexing complete.");
        }
        Commands::Embed { collection, force } => {
            println!("Generating embeddings... (collection: {:?}, force: {})", collection, force);
        }
        Commands::Search { query, count, collection, min_score } => {
            println!("BM25 Search for '{}' (count: {})", query, count);
            let conn = duckdb::Connection::open(&db_path)?;
            let results = qgoots_core::search::bm25_search(&conn, query, *count)?;
            for r in results {
                println!("[{:.2}] {} ({}): {}", r.score, r.title, r.path, r.snippet.replace('\n', " "));
            }
        }
        Commands::Vsearch { query, count, collection } => {
            println!("Vector Search for '{}'", query);
        }
        Commands::Query { query, count, collection, min_score } => {
            println!("Hybrid Search for '{}'", query);
        }
    }

    Ok(())
}
