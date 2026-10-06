#include <vulkan/vulkan.hpp>

int main() {
    uint32_t version = 0;
    return vkEnumerateInstanceVersion(&version) == VK_SUCCESS ? 0 : 1;
}
