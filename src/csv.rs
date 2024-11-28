use std::fs::File;
use anyhow::Result;
use std::io::BufWriter;
use crate::types::{Config, DownloadSpeedSet};
use crate::httping::HttpPing;
use crate::download::build_client;

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

        // 确定打印数量
        let print_num = std::cmp::min(self.len(), self.first().unwrap().config.print_num as usize);
        if print_num == 0 {
            return;
        }

        // 确定格式化字符串
        let (head_format, data_format) = {
            let has_ipv6 = self[..print_num]
                .iter()
                .any(|d| d.ping_data.ip.to_string().len() > 15);

            if has_ipv6 {
                ("%-40s%-5s%-5s%-5s%-6s%-11s%-8s\n", "%-42s%-8s%-8s%-8s%-10s%-15s%-8s\n")
            } else {
                ("%-16s%-5s%-5s%-5s%-6s%-11s%-8s\n", "%-18s%-8s%-8s%-8s%-10s%-15s%-8s\n")
            }
        };

        // 打印表头
        println!("{}", format!(
            "{} {} {} {} {} {} {} {}", head_format,
            "IP 地址", "已发送", "已接收", "丢包率", "平均延迟", "下载速度 (MB/s)", "数据中心"
        ));

        // 打印数据
        for ip_data in self.iter().take(print_num) {
            let data = ip_data.to_string_vec();
            println!("{}", format!(
                "{} {} {} {} {} {} {} {}", data_format,
                &data[0], &data[1], &data[2], &data[3], &data[4], &data[5], &data[6]
            ));
        }

        // 如果有输出文件，打印提示
        if !self[0].config.output.is_empty() {
            println!(
                "\n完整测速结果已写入 {} 文件，可使用记事本/表格软件查看。",
                self[0].config.output
            );
        }
    }
} 