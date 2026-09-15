/* The iOS bundle's entry point: hand over to the Rust library at once.
 * winit's iOS backend calls UIApplicationMain itself and never returns. */
extern void dicomscope_main(void);

int main(void) {
    dicomscope_main();
    return 0;
}
