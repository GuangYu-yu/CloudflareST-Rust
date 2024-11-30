use std::fs::File;
use anyhow::Result;
use std::io::BufWriter;
use crate::types::{Config, DownloadSpeedSet};
use crate::httping::HttpPing;
use crate::download::build_client;
use prettytable::{Table, Row, Cell, format};

pub async fn export_csv(data: &mut DownloadSpeedSet, config: &Config) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }

    let http_ping = HttpPing::new(config.clone(), None);
    for ip_data in data.iter_mut() {
        if let Some(client) = build_client(&ip_data.ping_data.ip, config).await {
            if let Ok(resp) = client.head(&config.url).send().await {
                if let Some(colo) = http_ping.get_colo(resp.headers()) {
                    ip_data.colo = colo;
                }
            }
        }
    }

    if config.output.is_empty() {
        return Ok(());
    }

    let file = File::create(&config.output)?;
    let buf_writer = BufWriter::with_capacity(32 * 1024, file);
    let mut writer = csv::Writer::from_writer(buf_writer);

    // 写入表头
    writer.write_record(&[
        "IP 地址",
        "已发送",
        "已接收", 
        "丢包率",
        "平均延迟",
        "下载速度 (MB/s)",
        "数据中心",
    ])?;

    // 写入数据
    for ip_data in data {
        writer.write_record(&ip_data.to_string_vec())?;
    }

    writer.flush()?;
    Ok(())
}

pub trait PrintResult {
    fn print(&self);
}

impl PrintResult for DownloadSpeedSet {
    fn print(&self) {
        if self.is_empty() {
            println!("\n[信息] 完整测速结果 IP 数量为 0，跳过输出结果。");
            return;
        }

        let mut table = Table::new();
        
        // 设置表格样式
        table.set_format(*format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
        
        // 添加表头，使用青色
        table.add_row(Row::new(vec![
            Cell::new("IP 地址").style_spec("Fc"),
            Cell::new("已发送").style_spec("Fc"),
            Cell::new("已接收").style_spec("Fc"),
            Cell::new("丢包率").style_spec("Fc"),
            Cell::new("平均延迟").style_spec("Fc"),
            Cell::new("下载速度 (MB/s)").style_spec("Fc"),
            Cell::new("数据中心").style_spec("Fc"),
        ]));

        // 添加数据行
        for ip_data in self.iter().take(self[0].config.print_num.try_into().unwrap()) {
            table.add_row(Row::new(vec![
                Cell::new(&ip_data.ping_data.ip.to_string()),
                Cell::new(&ip_data.ping_data.sended.to_string()),
                Cell::new(&ip_data.ping_data.received.to_string()),
                Cell::new(&format!("{:.2}", ip_data.loss_rate)),
                Cell::new(&format!("{:.2}", ip_data.ping_data.delay.as_millis())),
                Cell::new(&format!("{:.2}", ip_data.download_speed / 1024.0 / 1024.0)),
                Cell::new(&ip_data.colo),
            ]));
        }

        // 打印表格
        table.printstd();

        // 如果有输出文件，打印提示
        if !self[0].config.output.is_empty() {
            println!(
                "\n完整测速结果已写入 {} 文件，可使用记事本/表格软件查看。",
                self[0].config.output
            );
        }
    }
} 