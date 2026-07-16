use crate::args::Args;
use crate::common::PingData;
use crate::info_println;
use std::io::Write as IoWrite;

#[cfg(target_os = "windows")]
use std::io::Seek;

const TABLE_HEADERS: [&str; 7] = [
    "IP 地址",
    "已发送",
    "已接收",
    "丢包率",
    "平均延迟",
    "下载速度(MB/s)",
    "数据中心",
];

/// 从 PingResult 导出 CSV 文件
pub(crate) fn export_csv(results: &[PingData], args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    /// 写入CSV行到文件
    fn write_csv_line(file: &mut std::fs::File, fields: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        let line = fields.join(",");
        writeln!(file, "{}", line)?;
        Ok(())
    }

    if results.is_empty() || args.output.as_ref().is_none() {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    let mut file = crate::args::OUTPUT_HANDLE.get().unwrap().try_clone()?;
    #[cfg(target_os = "windows")]
    file.set_len(0)?;
    #[cfg(target_os = "windows")]
    file.rewind()?;

    #[cfg(not(target_os = "windows"))]
    let mut file = std::fs::File::create(args.output.as_ref().unwrap())?;

    write_csv_line(&mut file, &TABLE_HEADERS.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;

    for result in results {
        let mut record = ping_data_to_fields(result);
        record[0] = result.display_addr(args.show_port);
        write_csv_line(&mut file, &record)?;
    }

    file.flush()?;
    Ok(())
}

/// 打印测速结果
pub(crate) fn print_results(results: &[PingData], args: &Args) {
    if results.is_empty() {
        info_println(format_args!("测速结果 IP 数量为 0，跳过输出结果"));
        return;
    }

    const COLUMN_PADDING: usize = 3;
    const LEADING_SPACES: usize = 1;

    let print_num = results.len().min(args.print_num.into());

    let header_display_widths: [usize; 7] = TABLE_HEADERS.map(|s| s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum());
    let mut column_widths = header_display_widths.to_vec();

    let rows: Vec<Vec<String>> = results.iter()
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

    let base_width: usize = {
        let sum_content_widths: usize = column_widths.iter().sum();
        let sum_padding: usize = COLUMN_PADDING * (column_widths.len().saturating_sub(1));
        sum_content_widths + sum_padding + LEADING_SPACES
    };

    let leading = " ".to_string();
    let line = "─".repeat(base_width.saturating_sub(LEADING_SPACES));

    println!("{leading}{line}");

    print!("{leading}");
    for (i, header) in TABLE_HEADERS.iter().enumerate() {
        let pad = column_widths[i]
            .saturating_sub(header_display_widths[i]) + COLUMN_PADDING;
        print!("\x1b[1;97;100m{}\x1b[0m{}", header, " ".repeat(pad));
    }
    println!();

    for row in &rows {
        print!("{leading}");
        for (i, field) in row.iter().enumerate() {
            let pad = column_widths[i]
                .saturating_sub(field.chars().count()) + COLUMN_PADDING;
            print!("{}{}", field, " ".repeat(pad));
        }
        println!();
    }

    println!("{leading}{line}");
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
        data.colo_str().to_string(),
    ]
}