// Load only the caller's exact built framework, not a search-path fallback.
#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#include <stdio.h>

int main(int argc, const char *argv[])
{
    @autoreleasepool {
        if (argc != 2) { return 2; }
        NSString *path = [NSString stringWithUTF8String:argv[1]];
        if (![path isAbsolutePath]) { return 2; }
        NSBundle *framework = [NSBundle bundleWithPath:path];
        NSError *error = nil;
        if (framework == nil || ![framework loadAndReturnError:&error]) {
            NSLog(@"Cannot load framework: %@", error);
            return 1;
        }
        Class updater = NSClassFromString(@"SPUUpdater");
        SEL selector = NSSelectorFromString(@"explicitInstallationCommitVersion");
        Method method = class_getClassMethod(updater, selector);
        if (method == NULL) { return 1; } // Stock framework must fail here.
        NSUInteger version = ((NSUInteger (*)(id, SEL))method_getImplementation(method))(updater, selector);
        if (version != 1 || ![[NSBundle bundleForClass:updater].bundleURL.URLByResolvingSymlinksInPath.path isEqualToString:framework.bundleURL.URLByResolvingSymlinksInPath.path]) {
            return 1;
        }
        puts("PASS: loaded exact framework; explicitInstallationCommitVersion=1");
        return 0;
    }
}
