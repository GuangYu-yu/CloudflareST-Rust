use std::fs::File;
use std::io::{self, BufWriter};
use tabled::{Table, Tabled};
use tabled::settings::{Style, Modify, object::Rows};
use crate::args::Args;
use crate::PingData;

/// 辅助宏：计算字段个数
macro_rules! count_idents {
    ($($idents:ident),*) => {
        <[()]>::len(&[$(count_idents!(@sub $idents)),*])
    };
    (@sub $ident:ident) => { () };
}

/// 宏：一次性定义结构体和对应表头数组
macro_rules! define_result_struct {
    (
        $name:ident,
        $( ($field:ident, $title:expr) ),+ $(,)?
    ) => {
        #[derive(Tabled)]
        pub struct $name {
            $(
                #[tabled(rename = $title)]
                pub $field: String,
            )+
        }

        pub const TABLE_HEADERS: [&str; count_idents!($($field),+)] = [
            $($title),+
        ];
    };
}

// 使用宏定义 ResultData 结构体和 TABLE_HEADERS
define_result_struct!(
    ResultData,
    (ip_addr, "IP 地址"),
    (sent, "已发送"),
    (received, "已接收"),
    (loss_rate, "丢包率"),
    (delay, "平均延迟"),
    (download_speed, "下载速度(MB/s)"),
    (data_center, "数据中心"),
);

/// 定义结果打印 trait
pub trait PrintResult {
    fn print(&self, args: &Args);
}

/// 从 PingResult 导出 CSV 文件
pub fn export_csv(results: &[PingData], args: &Args) -> io::Result<()> {
    if results.is_empty() || args.output.is_empty() {
        return Ok(());
    }

    let file = File::create(&args.output)?;
    let mut writer = csv::Writer::from_writer(BufWriter::with_capacity(32 * 1024, file));

    // 写入表头（用宏自动生成的常量）
    writer.write_record(&TABLE_HEADERS)?;

    // 写入数据
    for result in results {
        let mut record = ping_data_to_fields(result);
        record[0] = result.display_addr(args.show_port);
        writer.write_record(&record)?;
    }

    writer.flush()?;
    Ok(())
}

impl PrintResult for Vec<PingData> {
    fn print(&self, args: &Args) {
        if self.is_empty() {
            println!("\n[信息] 测速结果 IP 数量为 0，跳过输出结果");
            return;
        }

        let mut results_data = Vec::new();
        
        for result in self.iter().take(args.print_num.into()) {
            let fields = ping_data_to_fields(result);
            results_data.push(ResultData {
                ip_addr: result.display_addr(args.show_port),
                sent: fields[1].clone(),
                received: fields[2].clone(),
                loss_rate: fields[3].clone(),
                delay: fields[4].clone(),
                download_speed: fields[5].clone(),
                data_center: fields[6].clone(),
            });
        }

        let mut table = Table::new(results_data);
        table
            .with(Style::blank())  // 去掉所有边框
            .with(Modify::new(Rows::first()).with(tabled::settings::Color::FG_CYAN)); // 表头青色
        println!("{}", table);

        if !args.output.is_empty() {
            println!("\n[信息] 测速结果已写入 {} 文件，可使用记事本/表格软件查看", args.output);
        }
    }
}

/// 将 PingData 转换为字符串字段
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