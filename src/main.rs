use macroquad::prelude::*;

#[macroquad::main("Hello Macroquad")]
async fn main() {
    loop {
        clear_background(DARKBLUE);

        draw_text("Hello, world!", 20.0, 60.0, 60.0, WHITE);
        draw_circle(screen_width() / 2.0, screen_height() / 2.0, 50.0, YELLOW);

        next_frame().await;
    }
}
