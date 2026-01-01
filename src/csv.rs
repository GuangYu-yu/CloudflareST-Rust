use crate::args::Args;
use crate::common::PingData;
use crate::info_println;
use std::io::Write as IoWrite;

const TABLE_HEADERS: [&str; 7] = [
    "IP 地址",
    "已发送",
    "已接收",
    "丢包率",
    "平均延迟",
    "下载速度(MB/s)",
    "数据中心",
];

/// 定义结果打印 trait
pub(crate) trait PrintResult {
    fn print(&self, args: &Args);
}

/// 从 PingResult 导出 CSV 文件
pub(crate) fn export_csv(results: &[PingData], args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    /// 写入CSV行到文件
    fn write_csv_line(file: &mut std::fs::File, fields: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        let line = fields.join(",");
        writeln!(file, "{}", line)?;
        Ok(())
    }

    // 如果没有结果或未指定输出文件，直接返回
    if results.is_empty() || args.output.as_ref().is_none() {
        return Ok(());
    }

    let file_path = args.output.as_ref().unwrap();
    let mut file = std::fs::File::create(file_path)?;

    // 写入表头
    write_csv_line(&mut file, &TABLE_HEADERS.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;

    // 写入数据
    for result in results {
        let mut record = ping_data_to_fields(result);
        record[0] = result.display_addr(args.show_port);
        write_csv_line(&mut file, &record)?;
    }

    // 确保数据写入磁盘
    file.flush()?;

    Ok(())
}

impl PrintResult for Vec<PingData> {
    fn print(&self, args: &Args) {
        if self.is_empty() {
            info_println(format_args!("测速结果 IP 数量为 0，跳过输出结果"));
            return;
        }

        const COLUMN_PADDING: usize = 3; // 每列额外间距
        const LEADING_SPACES: usize = 1; // 前导空格数量

        let print_num = self.len().min(args.print_num.into());

        // 初始列宽来自固定表头宽度
        let header_display_widths = [7, 6, 6, 6, 8, 14, 8];
        let mut column_widths = header_display_widths.to_vec();

        // 预计算每行数据并动态更新列宽
        let rows: Vec<Vec<String>> = self.iter()
            .take(print_num)
            .map(|r| {
                ping_data_to_fields(r)
                    .into_iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let display = if i == 0 { r.display_addr(args.show_port) } else { f };
                        column_widths[i] = column_widths[i].max(display.chars().count());
                        display
                    })
                    .collect()
            })
            .collect();

        // 分割线宽度
        let base_width: usize = {
            let sum_content_widths: usize = column_widths.iter().sum();
            let sum_padding: usize = COLUMN_PADDING * (column_widths.len().saturating_sub(1));
            sum_content_widths + sum_padding + LEADING_SPACES
        };

        let leading = " ".to_string();
        let line = "─".repeat(base_width.saturating_sub(LEADING_SPACES));

        // 输出分割线
        println!("{leading}{line}");

        // 表头
        print!("{leading}");
        for (i, header) in TABLE_HEADERS.iter().enumerate() {
            let pad = column_widths[i]
                .saturating_sub(header_display_widths[i]) + COLUMN_PADDING;
            print!("\x1b[1;97;100m{}\x1b[0m{}", header, " ".repeat(pad));
        }
        println!();

        // 数据行
        for row in &rows {
            print!("{leading}");
            for (i, field) in row.iter().enumerate() {
                let pad = column_widths[i]
                    .saturating_sub(field.chars().count()) + COLUMN_PADDING;
                print!("{}{}", field, " ".repeat(pad));
            }
            println!();
        }

        // 尾部分割线
        println!("{leading}{line}");
    }
}

/// 将 PingData 转换为通用数据格式
fn ping_data_to_fields(data: &PingData) -> Vec<String> {
    vec![
        data.addr.to_string(),
        data.sent.to_string(),
        data.received.to_string(),
        format!("{:.2}", data.loss_rate()),
        format!("{:.2}", data.delay),
        match data.download_speed {
            Some(speed) => format!("{:.2}", speed / 1024.0 / 1024.0),
            None => String::new(),
        },
        data.data_center.to_string(),
    ]
}